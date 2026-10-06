use std::os::unix::process::CommandExt;
use std::process::Command as Process;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use dialoguer::{Confirm, FuzzySelect};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::Config;
use crate::{platform, run};

const SSM_POLICY_ARN: &str = "arn:aws:iam::aws:policy/AmazonSSMManagedInstanceCore";
const INSTANCE_QUERY: &str = "Reservations[].Instances[].{id: InstanceId, name: Tags[?Key=='Name'] | [0].Value, state: State.Name, profile: IamInstanceProfile.Arn}";

#[derive(Subcommand)]
pub enum Command {
    /// Pick an EC2 instance and open an SSM shell on it.
    Connect(AwsArgs),
    /// Attach AmazonSSMManagedInstanceCore to an instance's IAM role. This writes to IAM.
    Enable(AwsArgs),
}

#[derive(Args)]
pub struct AwsArgs {
    /// AWS region. Falls back to `aws.region` in the config, then to the AWS CLI default.
    #[arg(short, long, env = "AWS_REGION")]
    region: Option<String>,
    /// AWS profile. Falls back to `aws.profile` in the config.
    #[arg(short, long, env = "AWS_PROFILE")]
    profile: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Instance {
    id: String,
    name: Option<String>,
    state: String,
    profile: Option<String>,
}

impl Instance {
    fn label(&self) -> String {
        format!(
            "{} ({}) [{}]",
            self.name.as_deref().unwrap_or("-"),
            self.id,
            self.state
        )
    }
}

struct Aws {
    region: Option<String>,
    profile: Option<String>,
}

impl Aws {
    fn new(args: AwsArgs, cfg: &Config) -> Result<Self> {
        platform::require("aws")?;
        Ok(Self {
            region: args.region.or_else(|| cfg.aws.region.clone()),
            profile: args.profile.or_else(|| cfg.aws.profile.clone()),
        })
    }

    fn cmd(&self) -> Process {
        let mut cmd = Process::new("aws");
        if let Some(region) = &self.region {
            cmd.args(["--region", region]);
        }
        if let Some(profile) = &self.profile {
            cmd.args(["--profile", profile]);
        }
        cmd
    }

    fn json<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T> {
        let out = run::capture(self.cmd().args(args).args(["--output", "json"]))?;
        serde_json::from_str(&out).context("unexpected AWS CLI output")
    }

    fn region_label(&self) -> String {
        match &self.region {
            Some(region) => format!("region {region}"),
            None => "the AWS CLI default region".to_string(),
        }
    }

    fn instances(&self) -> Result<Vec<Instance>> {
        eprintln!("Fetching EC2 instances in {}...", self.region_label());
        self.json(&[
            "ec2",
            "describe-instances",
            "--filters",
            "Name=instance-state-name,Values=pending,running,stopping,stopped",
            "--query",
            INSTANCE_QUERY,
        ])
    }
}

pub fn run(cmd: Command, cfg: &Config) -> Result<()> {
    match cmd {
        Command::Connect(args) => connect(&Aws::new(args, cfg)?),
        Command::Enable(args) => enable(&Aws::new(args, cfg)?),
    }
}

fn pick<'a>(instances: &'a [Instance], prompt: &str) -> Result<Option<&'a Instance>> {
    let index = FuzzySelect::new()
        .with_prompt(format!("{prompt} (type to filter, Esc to cancel)"))
        .items(instances.iter().map(Instance::label))
        .default(0)
        .interact_opt()?;
    Ok(index.map(|i| &instances[i]))
}

fn connect(aws: &Aws) -> Result<()> {
    platform::require("session-manager-plugin")?;
    let instances = aws.instances()?;
    if instances.is_empty() {
        println!("No EC2 instances found in {}.", aws.region_label());
        return Ok(());
    }
    let Some(instance) = pick(&instances, "Instance to connect to")? else {
        println!("Cancelled.");
        return Ok(());
    };

    eprintln!("Starting SSM session with {}...", instance.label());
    let err = aws
        .cmd()
        .args(["ssm", "start-session", "--target", &instance.id])
        .exec();
    Err(err).context("failed to run `aws ssm start-session`")
}

fn enable(aws: &Aws) -> Result<()> {
    let instances = aws.instances()?;
    if instances.is_empty() {
        println!("No EC2 instances found in {}.", aws.region_label());
        return Ok(());
    }
    let Some(instance) = pick(&instances, "Instance to enable SSM on")? else {
        println!("Cancelled.");
        return Ok(());
    };

    let profile_arn = instance.profile.as_deref().with_context(|| {
        format!(
            "{} has no IAM instance profile. Associate one first, then re-run to attach AmazonSSMManagedInstanceCore",
            instance.label()
        )
    })?;
    let role: Option<String> = aws.json(&[
        "iam",
        "get-instance-profile",
        "--instance-profile-name",
        profile_name(profile_arn),
        "--query",
        "InstanceProfile.Roles[0].RoleName",
    ])?;
    let role = role.with_context(|| format!("instance profile {profile_arn} has no IAM role"))?;

    let attached: Vec<String> = aws.json(&[
        "iam",
        "list-attached-role-policies",
        "--role-name",
        &role,
        "--query",
        "AttachedPolicies[].PolicyArn",
    ])?;
    if attached.iter().any(|arn| arn == SSM_POLICY_ARN) {
        println!(
            "{} already has AmazonSSMManagedInstanceCore on role {role}.",
            instance.label()
        );
        return Ok(());
    }

    // The role is shared by every instance with this profile, so the change reaches all of them.
    let sharing = instances
        .iter()
        .filter(|i| i.profile.as_deref() == Some(profile_arn))
        .count();
    let confirmed = Confirm::new()
        .with_prompt(format!(
            "Attach AmazonSSMManagedInstanceCore to role {role}? {sharing} instance(s) in {} use this profile.",
            aws.region_label()
        ))
        .default(false)
        .interact()?;
    if !confirmed {
        println!("Skipped adding SSM permission.");
        return Ok(());
    }

    eprintln!("Attaching AmazonSSMManagedInstanceCore to role {role}...");
    run::status(aws.cmd().args([
        "iam",
        "attach-role-policy",
        "--role-name",
        &role,
        "--policy-arn",
        SSM_POLICY_ARN,
    ]))?;
    let region_flag = aws
        .region
        .as_ref()
        .map(|r| format!(" -r {r}"))
        .unwrap_or_default();
    println!(
        "SSM permission added. Wait 1 to 2 minutes for the agent, then connect with:\n  sts ssm connect{region_flag}"
    );
    Ok(())
}

/// Instance profile ARNs may carry a path: `arn:aws:iam::123:instance-profile/path/name`.
fn profile_name(arn: &str) -> &str {
    arn.rsplit('/').next().unwrap_or(arn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_describe_instances_output() {
        let instances: Vec<Instance> = serde_json::from_str(
            r#"[
                {"id":"i-0abc","name":"web-1","state":"running","profile":"arn:aws:iam::123456789012:instance-profile/web"},
                {"id":"i-0def","name":null,"state":"stopped","profile":null}
            ]"#,
        )
        .unwrap();
        assert_eq!(instances[0].label(), "web-1 (i-0abc) [running]");
        assert_eq!(instances[1].label(), "- (i-0def) [stopped]");
        assert!(instances[1].profile.is_none());
    }

    #[test]
    fn extracts_profile_name_from_arn() {
        assert_eq!(
            profile_name("arn:aws:iam::123456789012:instance-profile/web"),
            "web"
        );
        assert_eq!(
            profile_name("arn:aws:iam::123456789012:instance-profile/team/app/web"),
            "web"
        );
    }
}

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{bail, Context, Result};
use arguments::Operation;
use regex::Regex;
use tokio::process::Command;

use crate::firewall::firewall;

mod arguments;
mod deploy;
mod firewall;
mod ssh;

#[tokio::main]
async fn main() {
    let args = argh::from_env();

    colog::init();

    if let Err(error) = application(args).await {
        log::error!("Fatal error: {:?}", error);
    }
}

async fn application(args: arguments::RosAssistant) -> Result<()> {
    log::info!("Nix Home CLI v{}", std::env!("CARGO_PKG_VERSION"));

    match args.subcommand {
        arguments::SubCommand::NewProject(new_project_args) => {
            new_project(new_project_args).context("Failed to create new project")
        }
        arguments::SubCommand::Deploy(deploy_args) => {
            deploy::deploy(args.build_machine, deploy_args)
                .await
                .context("Failed to deploy project")
        }
        arguments::SubCommand::Ssh(ssh_args) => {
            ssh::ssh(ssh_args).await.context("Failed to ssh to host")
        }
        arguments::SubCommand::Firewall(firewall_args) => firewall(firewall_args).await,
    }
}

fn new_project(_args: arguments::NewProject) -> Result<()> {
    bail!("New project sub-command is not yet implemented.")
}

pub struct ProjectContext {
    build_machines: Vec<String>,
    host_filter: Regex,
    ssh_config_path: String,
    project_root: PathBuf,
    output_directory: PathBuf,
}

impl ProjectContext {
    async fn load_project(
        build_machines: Vec<String>,
        project_root: Option<PathBuf>,
        host_filter: Option<&str>,
        link_path: Option<&Path>,
    ) -> Result<Self> {
        let project_root = project_root.map(Ok).unwrap_or_else(|| {
            std::env::current_dir().context("Failed to get current directory")
        })?;

        log::info!("Project root: {:?}", project_root);

        let ssh_config = project_root.join("ssh_config");
        if !ssh_config.exists() {
            log::warn!("Project is missing `ssh_config` file. File will be created for you.");
            // It's fine for the default to just be empty.
            tokio::fs::write(&ssh_config, "")
                .await
                .context("Failed to create `ssh_config` file.")?;
        }

        Self::new(
            build_machines,
            host_filter,
            ssh_config,
            project_root,
            link_path,
        )
    }

    fn new(
        build_machines: Vec<String>,
        host_filter: Option<&str>,
        ssh_config: PathBuf,
        project_root: PathBuf,
        link_path: Option<&Path>,
    ) -> Result<Self> {
        log::info!("Project root: {:?}", project_root);

        if let Some(host_filter) = host_filter {
            log::info!("Host filter: '{host_filter}'");
        } else {
            log::info!("Host filter: None");
        }

        let host_filter = Regex::new(host_filter.unwrap_or(".*"))
            .context("Failed to compile regex expression for host filter")?;

        let ssh_config_path = ssh_config
            .as_os_str()
            .to_str()
            .map(|s| s.to_string())
            .context("Path to SSH config could not be encoded as UTF8")?;

        let output_directory = link_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| project_root.join("result"));
        if output_directory.exists() {
            if output_directory.is_dir() {
                std::fs::remove_dir_all(&output_directory)
            } else {
                std::fs::remove_file(&output_directory)
            }
            .context("Failed to remove old result output.")?;
        };

        Ok(Self {
            build_machines,
            host_filter,
            ssh_config_path,
            project_root,
            output_directory,
        })
    }

    /// Checks the system configuration if a field is present and set to true.
    /// If the value is not present, this returns false.
    async fn check_project_config_flag(&self, host: &str, field_path: &[&str]) -> Result<bool> {
        let mut command = Command::new("nix");
        command.args(["eval", "--read-only", "--apply"]);

        let mut iter = field_path.iter().peekable();
        let mut query = {
            let first = iter
                .next()
                .expect("Config flag needs a least one layer to the path");
            format!("{first} or false")
        };

        while let Some(layer) = iter.next() {
            if iter.peek().is_some() {
                query = format!("({query}).{layer} or false");
            } else {
                query = format!("(config.{query}).{layer} or false");
            }
        }

        command.arg(format!("config: {query}"));
        command.arg(format!(".#nixosConfigurations.{host}.config"));

        let result = command.output().await.context("Failed to run `nix eval`")?;
        let stderr = String::from_utf8_lossy(&result.stderr);
        if result.status.success() {
            let mut output = String::from_utf8(result.stdout)
                .context("`nix eval` output is not utf8 encoded text")?;
            output.retain(|c| c.is_alphabetic());

            dbg!(&output);
            let value: bool = output
                .parse()
                .context("Failed to parse output of `nix eval`")?;

            Ok(value)
        } else {
            bail!("`nix eval` returned status {}: {}", result.status, stderr);
        }
    }

    async fn get_hosts_list(&self) -> Result<Vec<String>> {
        let mut command = Command::new("nix");
        command.args([
            "eval",
            "--raw",
            ".#nixosConfigurations",
            "--apply",
            "pkgs: builtins.concatStringsSep \" \" (builtins.attrNames pkgs)",
        ]);

        let result = command.output().await.context("Failed to run `nix eval`")?;
        let stderr = String::from_utf8_lossy(&result.stderr);
        if result.status.success() {
            if !result.stderr.is_empty() {
                log::warn!("`nix eval` had stderr output: {}", stderr);
            }

            let output = String::from_utf8(result.stdout)
                .context("`nix eval` output is not utf8 encoded text")?;

            let hosts = output.split_whitespace();
            Ok(hosts.map(|s| s.to_string()).collect())
        } else {
            bail!("`nix eval` returned status {}: {}", result.status, stderr);
        }
    }

    async fn deploy_ssh(
        &self,
        host: &str,
        hostname: &str,
        operation: Operation,
        enable_auto_revert: bool,
    ) -> Result<()> {
        log::info!("Deploying {host} to {hostname}");

        if enable_auto_revert {
            if !self
                .check_project_config_flag(host, &["auto-revert", "enabled"])
                .await
                .context("Failed to check if host supports auto-revert")?
            {
                bail!("Configuration to deploy does not support auto-revert.\n\
                    Pass `--no-auto-revert` if you are CERTAIN you want to deploy without this.\n\
                    Add the `auto-revert.nix` module to your configuration to add auto-revert support.");
            }

            log::info!("Setting auto-revert timer using ssh.");
            self.run_ssh(
                host,
                Some("mkdir -p /run/rhome/auto-revert && touch /run/rhome/auto-revert/set"),
            )
            .await
            .context("Failed to run command to start auto-revert timer")?;
            log::info!("Timer will start on system activation.");
        } else {
            log::warn!("Auto-revert timer has been disabled for this deployment");
        }

        let mut command = Command::new("nixos-rebuild");
        command.env("NIX_SSHOPTS", format!("-F {}", self.ssh_config_path));
        command.current_dir(&self.project_root);

        // Configure builders.
        let build_machine_list = self.build_machines.join(";");
        command.arg("--builders");
        command.arg(build_machine_list);

        // What kind of operation are we doing?
        match operation {
            Operation::Switch => command.arg("switch"),
            Operation::Test => command.arg("test"),
            Operation::Boot => command.arg("boot"),
        };

        // Configure target host.
        command.arg("--flake");
        command.arg(format!(".#{host}"));

        command.arg("--target-host");
        command.arg(hostname);

        let mut child = command
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn nixos-rebuild.")?;

        let result = child
            .wait()
            .await
            .context("Failed to wait for nixos-rebuild to complete.")?;

        if !result.success() {
            bail!("`nixos-rebuild` returned non-zero output.");
        } else {
            // Cancel the auto-revert.
            if enable_auto_revert {
                log::info!("Cancelling auto-revert timer using ssh.");
                self.run_ssh(host, Some("rm -f /run/rhome/auto-revert/set"))
                    .await
                    .context("Failed to run command to stop auto-revert timer")?;
            }
            Ok(())
        }
    }

    async fn run_build(&self, host: &str, target: &str) -> Result<PathBuf> {
        log::info!("Building '{}'", host);

        let mut command = Command::new("nix");
        command.env("NIX_SSHOPTS", format!("-F {}", self.ssh_config_path));
        command.current_dir(&self.project_root);

        // Configure builders.
        let build_machine_list = self.build_machines.join(";");
        command.arg("--builders");
        command.arg(build_machine_list);

        // Configure output path.
        let output_directory = self.output_directory.join(host);

        // Our action.
        command.arg("build");

        command.arg("--out-link");
        command.arg(output_directory.clone());

        // Specify which output to build.
        command.arg(target);

        let mut child = command
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Failed to spawn nix build.")?;

        let result = child
            .wait()
            .await
            .context("Failed to wait for nix-build to complete.")?;

        if !result.success() {
            bail!("`nix build` returned non-zero output.");
        } else {
            Ok(output_directory)
        }
    }

    async fn run_against_hosts(
        &self,
        list_check: impl Fn(&[String]) -> Result<()>,
        mut to_run: impl AsyncFnMut(&str) -> Result<()>,
    ) -> Result<()> {
        let mut host_list = self
            .get_hosts_list()
            .await
            .context("Failed to get list of hosts from flake.nix")?;

        host_list.retain(move |host| self.host_filter.captures(host).is_some());

        list_check(&host_list)?;

        for host in host_list.iter() {
            to_run(host)
                .await
                .with_context(|| format!("Error while processing host {host}"))?;
        }

        Ok(())
    }

    async fn select_default_host(&self) -> Result<String> {
        let mut hosts = self
            .get_hosts_list()
            .await
            .context("Failed to get host list")?;
        let host = hosts.pop();

        // If there's only one host on the robot, just assume it's that one.
        if hosts.is_empty() {
            if let Some(host) = host {
                Ok(host)
            } else {
                bail!("No hosts available for this robot. Please add a host configuration to flake.nix");
            }
        } else {
            bail!(
                "Multiple hosts are available for this robot. Select one with the `--host` argument."
            );
        }
    }

    async fn run_ssh(&self, host: &str, arg: Option<&str>) -> Result<()> {
        let mut command = Command::new("ssh");
        command.arg("-F");
        command.arg(&self.ssh_config_path);
        command.arg(host);

        // We can automatically run a command.
        // If no argument is provided, we will spawn an interactive terminal.
        if let Some(arg) = arg {
            command.arg(arg);
        }

        let mut child = command
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::inherit())
            .spawn()
            .context("Failed to spawn ssh.")?;

        let result = child
            .wait()
            .await
            .context("Failed to wait for ssh to complete.")?;

        if !result.success() {
            bail!("Ssh unsuccessful.");
        } else {
            log::info!("Ssh successful.");
            Ok(())
        }
    }
}

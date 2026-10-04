use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum, ValueHint};

#[derive(Parser, PartialEq, Debug)]
#[command(name = "nhome")]
/// Manages robot workspaces, deployment, and integration with Home Assistant.
pub struct RosAssistant {
    #[arg(short = 'b', long, action = ArgAction::Append)]
    /// specify a remote build machine to be used to build your project. This is especially useful for cross compiling.
    /// specify each machine as `--build-machine 'ssh://hostname x86_64-linux aarch64-linux'`, adjusting the hostname
    /// and supported architectures as needed.
    pub build_machine: Vec<String>,

    #[command(subcommand)]
    pub subcommand: SubCommand,
}

#[derive(Subcommand, PartialEq, Debug)]
pub enum SubCommand {
    #[command(name = "new")]
    NewProject(NewProject),
    Deploy(Deploy),
    Ssh(SshCommand),
    Firewall(firewall::Command),
    #[command(name = "__hosts", hide = true)]
    Hosts(Hosts),
    #[command(name = "__completions", hide = true)]
    Completions(Completions),
}

#[derive(Args, PartialEq, Debug)]
/// Create a new robot project.
pub struct NewProject {}

#[derive(Args, PartialEq, Debug)]
/// Build and deploy a project.
pub struct Deploy {
    #[arg(long)]
    /// restrict which hosts are deployed using a regex expression
    pub hosts: Option<String>,

    #[arg(long, value_hint = ValueHint::DirPath)]
    /// specify a directory to be used as the project root (defaults to the current directory)
    pub project_root: Option<PathBuf>,

    #[command(subcommand)]
    pub deploy_type: DeployType,
}

#[derive(Subcommand, PartialEq, Debug)]
pub enum DeployType {
    Ssh(SshDeploy),
    #[command(name = "disk")]
    DiskImage(DiskImage),
    #[command(name = "install-iso")]
    InstallerIso(InstallISO),
    #[command(name = "install-netboot")]
    Netboot(InstallNetboot),
    LxcTemplate(LxcTemplate),
}

#[derive(Args, PartialEq, Debug)]
/// Build and deploy a project over ssh.
pub struct SshDeploy {
    #[arg(value_enum, default_value_t)]
    /// deployment operation: test (default), switch, boot
    pub operation: Operation,

    /// do not trigger the auto-revert timer (this has a risk of locking you out of your robot if
    /// things go wrong)
    #[arg(long)]
    pub no_auto_revert: bool,

    #[arg(long)]
    /// override the default ssh destination (only works if deploying to a single host)
    pub destination: Option<String>,
}

#[derive(ValueEnum, PartialEq, Debug, Clone, Copy)]
pub enum Operation {
    /// makes the configuration the new boot default and switches to it
    Switch,

    /// deploy and switch to the new configuration, but do not make it a boot entry so that
    /// rebooting will undo the changes
    Test,

    /// makes the configuration the new boot default but do not switch to it until reboot
    Boot,
}

impl Default for Operation {
    fn default() -> Self {
        Self::Test
    }
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Switch => "switch",
            Self::Test => "test",
            Self::Boot => "boot",
        };
        f.write_str(s)
    }
}

#[derive(Args, PartialEq, Debug)]
/// Build a project and create an initaial boot disk image for it.
pub struct DiskImage {
    #[arg(long, value_hint = ValueHint::DirPath)]
    /// override the default link path for the project
    pub link_path: Option<PathBuf>,
}

#[derive(Args, PartialEq, Debug)]
/// Build an ISO image for performing unattended installations of the disk image.
/// This image can be written to a USB drive or burned to a CD/DVD. Note that this
/// image is DESTRUCTIVE to any machine it is deployed on, as it will overwrite any
/// content on the target hard drive.
pub struct InstallISO {
    #[arg(long, value_hint = ValueHint::DirPath)]
    /// override the default link path for the project
    pub link_path: Option<PathBuf>,
}

#[derive(Args, PartialEq, Debug)]
/// Build an LXC template container image (tar.xz) for use with Proxmox VE.
pub struct LxcTemplate {
    #[arg(long, value_hint = ValueHint::DirPath)]
    /// override the default link path for the project
    pub link_path: Option<PathBuf>,
}

#[derive(Args, PartialEq, Debug)]
/// Build an ISO image for performing unattended installations of the disk image.
/// This image can be written to a USB drive or burned to a CD/DVD. Note that this
/// image is DESTRUCTIVE to any machine it is deployed on, as it will overwrite any
/// content on the target hard drive.
pub struct InstallNetboot {
    #[command(subcommand)]
    pub steps: netboot::Steps,
}

pub mod netboot {
    use super::*;

    #[derive(Subcommand, PartialEq, Debug)]
    pub enum Steps {
        Both(Both),
        Boot(Boot),
        Install(Install),
    }

    #[derive(Args, PartialEq, Debug)]
    /// PXE boot the robot, and then install to it.
    pub struct Both {}

    #[derive(Args, PartialEq, Debug)]
    /// PXE boot the robot.
    pub struct Boot {}

    #[derive(Args, PartialEq, Debug)]
    /// Install to a robot that has already been PXE booted.
    pub struct Install {
        #[arg(long)]
        /// override the default ssh destination (only works if deploying to a single host)
        pub destination: Option<String>,
    }
}

#[derive(Args, PartialEq, Debug)]
/// Ssh into your robot's computer.
pub struct SshCommand {
    #[arg(long, value_hint = ValueHint::DirPath)]
    /// specify a directory to be used as the project root (defaults to the current directory)
    pub project_root: Option<PathBuf>,

    #[arg(value_hint = ValueHint::Other)]
    pub host: Option<String>,

    #[arg(short = 'c', long)]
    /// run a command on the host.
    pub command: Option<String>,
}

#[derive(Args, PartialEq, Debug)]
/// List the host names of the project in the given (or current) directory, one per line.
/// Internal subcommand used by the shell tab-completion script; not intended for humans.
pub struct Hosts {
    #[arg(long, value_hint = ValueHint::DirPath)]
    /// specify a directory to be used as the project root (defaults to the current directory)
    pub project_root: Option<PathBuf>,
}

#[derive(Args, PartialEq, Debug)]
/// Print shell tab-completion scripts.
/// Internal subcommand used to generate the shipped completion file; not intended for humans.
pub struct Completions {
    /// the shell to print completions for
    #[arg(value_enum, default_value_t)]
    pub shell: CompletionShell,
}

#[derive(ValueEnum, PartialEq, Debug, Clone, Copy)]
pub enum CompletionShell {
    Bash,
}

impl Default for CompletionShell {
    fn default() -> Self {
        Self::Bash
    }
}

impl std::fmt::Display for CompletionShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Bash => "bash",
        };
        f.write_str(s)
    }
}

pub mod firewall {
    use super::*;

    #[derive(Args, PartialEq, Debug)]
    /// Manage the robot's firewalls.
    pub struct Command {
        #[arg(long)]
        /// restrict which hosts are modified using a regex expression
        pub hosts: Option<String>,

        #[arg(long, value_hint = ValueHint::DirPath)]
        /// specify a directory to be used as the project root (defaults to the current directory)
        pub project_root: Option<PathBuf>,

        #[command(subcommand)]
        pub subcommand: SubCommand,
    }

    #[derive(Subcommand, PartialEq, Debug)]
    pub enum SubCommand {
        Disable(Disable),
        Reset(Reset),
        Pierce(Pierce),
    }

    #[derive(Args, PartialEq, Debug)]
    /// Disable the firewalls.
    pub struct Disable {}

    #[derive(Args, PartialEq, Debug)]
    /// Reset the firewalls to their original state.
    pub struct Reset {}

    #[derive(Args, PartialEq, Debug)]
    /// Creates an opening in the firewalls just to your local system.
    pub struct Pierce {
        #[arg(long, action = ArgAction::Append)]
        /// specify an IP address or host name to open the firewalls to. You can use a hostname instead of an IP address.
        /// All addresses that hostname resolves to will be used. Do not specify any hosts to assume the addresses of all non-loopback
        /// network interfaces of this computer.
        pub host: Vec<String>,
    }
}

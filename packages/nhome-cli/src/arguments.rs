use std::path::PathBuf;

use argh::{FromArgValue, FromArgs};

#[derive(FromArgs, PartialEq, Debug)]
/// Manages robot workspaces, deployment, and integration with Home Assistant.
pub struct RosAssistant {
    #[argh(option, short = 'b')]
    /// specify a remote build machine to be used to build your project. This is especially useful for cross compiling.
    /// specify each machine as `--build-machine 'ssh://hostname x86_64-linux aarch64-linux'`, adjusting the hostname
    /// and supported architectures as needed.
    pub build_machine: Vec<String>,

    #[argh(subcommand)]
    pub subcommand: SubCommand,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub enum SubCommand {
    NewProject(NewProject),
    Deploy(Deploy),
    Ssh(SshCommand),
    Firewall(firewall::Command),
}

#[derive(FromArgs, PartialEq, Debug)]
/// Create a new robot project.
#[argh(subcommand, name = "new")]
pub struct NewProject {}

#[derive(FromArgs, PartialEq, Debug)]
/// Build and deploy a project.
#[argh(subcommand, name = "deploy")]
pub struct Deploy {
    #[argh(option)]
    /// restrict which hosts are deployed using a regex expression
    pub hosts: Option<String>,

    #[argh(option)]
    /// specify a directory to be used as the project root (defaults to the current directory)
    pub project_root: Option<PathBuf>,

    #[argh(subcommand)]
    pub deploy_type: DeployType,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
pub enum DeployType {
    Ssh(SshDeploy),
    DiskImage(DiskImage),
    InstallerIso(InstallISO),
    Netboot(InstallNetboot),
}

#[derive(FromArgs, PartialEq, Debug)]
/// Build and deploy a project over ssh.
#[argh(subcommand, name = "ssh")]
pub struct SshDeploy {
    #[argh(positional, default = "Operation::default()")]
    /// deployment operation: test (default), switch, boot
    pub operation: Operation,

    /// do not trigger the auto-revert timer (this has a risk of locking you out of your robot if
    /// things go wrong)
    #[argh(switch)]
    pub no_auto_revert: bool,

    #[argh(option)]
    /// override the default ssh destination (only works if deploying to a single host)
    pub destination: Option<String>,
}

#[derive(FromArgValue, PartialEq, Debug, Clone, Copy)]
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

#[derive(FromArgs, PartialEq, Debug)]
/// Build a project and create an initaial boot disk image for it.
#[argh(subcommand, name = "disk")]
pub struct DiskImage {
    #[argh(option)]
    /// override the default link path for the project
    pub link_path: Option<PathBuf>,
}

#[derive(FromArgs, PartialEq, Debug)]
/// Build an ISO image for performing unattended installations of the disk image.
/// This image can be written to a USB drive or burned to a CD/DVD. Note that this
/// image is DESTRUCTIVE to any machine it is deployed on, as it will overwrite any
/// content on the target hard drive.
#[argh(subcommand, name = "install-iso")]
pub struct InstallISO {
    #[argh(option)]
    /// override the default link path for the project
    pub link_path: Option<PathBuf>,
}

#[derive(FromArgs, PartialEq, Debug)]
/// Build an ISO image for performing unattended installations of the disk image.
/// This image can be written to a USB drive or burned to a CD/DVD. Note that this
/// image is DESTRUCTIVE to any machine it is deployed on, as it will overwrite any
/// content on the target hard drive.
#[argh(subcommand, name = "install-netboot")]
pub struct InstallNetboot {}

#[derive(FromArgs, PartialEq, Debug)]
/// Ssh into your robot's computer.
#[argh(subcommand, name = "ssh")]
pub struct SshCommand {
    #[argh(option)]
    /// specify a directory to be used as the project root (defaults to the current directory)
    pub project_root: Option<PathBuf>,

    #[argh(positional)]
    pub host: Option<String>,

    #[argh(option, short = 'c')]
    /// run a command on the host.
    pub command: Option<String>,
}

pub mod firewall {
    use super::*;

    #[derive(FromArgs, PartialEq, Debug)]
    /// Manage the robot's firewalls.
    #[argh(subcommand, name = "firewall")]
    pub struct Command {
        #[argh(option)]
        /// restrict which hosts are modified using a regex expression
        pub hosts: Option<String>,

        #[argh(option)]
        /// specify a directory to be used as the project root (defaults to the current directory)
        pub project_root: Option<PathBuf>,

        #[argh(subcommand)]
        pub subcommand: SubCommand,
    }

    #[derive(FromArgs, PartialEq, Debug)]
    #[argh(subcommand)]
    pub enum SubCommand {
        Disable(Disable),
        Reset(Reset),
        Pierce(Pierce),
    }

    #[derive(FromArgs, PartialEq, Debug)]
    /// Disable the firewalls.
    #[argh(subcommand, name = "disable")]
    pub struct Disable {}

    #[derive(FromArgs, PartialEq, Debug)]
    /// Reset the firewalls to their original state.
    #[argh(subcommand, name = "reset")]
    pub struct Reset {}

    #[derive(FromArgs, PartialEq, Debug)]
    /// Creates an opening in the firewalls just to your local system.
    #[argh(subcommand, name = "pierce")]
    pub struct Pierce {
        #[argh(option)]
        /// specify an IP address or host name to open the firewalls to. You can use a hostname instead of an IP address.
        /// All addresses that hostname resolves to will be used. Do not specify any hosts to assume the addresses of all non-loopback
        /// network interfaces of this computer.
        pub host: Vec<String>,
    }
}

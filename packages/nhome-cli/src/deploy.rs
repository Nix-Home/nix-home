use std::path::PathBuf;

use crate::{arguments, ProjectContext};
use anyhow::{bail, Context, Result};
use tokio::process::Command;

pub async fn deploy(build_machines: Vec<String>, args: arguments::Deploy) -> Result<()> {
    let host_filter = args.hosts.as_ref().map(|s| s.as_str());
    if let Some(host_filter) = host_filter {
        log::info!("Using host filter `{host_filter}`");
    }

    match args.deploy_type {
        arguments::DeployType::Ssh(ssh_args) => {
            deploy_ssh(build_machines, args.project_root, host_filter, ssh_args)
                .await
                .context("Failed to deploy")
        }
        arguments::DeployType::DiskImage(disk_args) => {
            build_disk_images(build_machines, args.project_root, host_filter, disk_args)
                .await
                .context("Failed to build disk image")
        }
        arguments::DeployType::InstallerIso(iso_args) => {
            build_iso_installer(build_machines, args.project_root, host_filter, iso_args)
                .await
                .context("Failed to build installer iso image")
        }
        arguments::DeployType::Netboot(netboot) => {
            install_netboot(build_machines, args.project_root, host_filter, netboot)
                .await
                .context("Failed to deploy")
        }
    }
}

async fn deploy_ssh<'a>(
    build_machines: Vec<String>,
    project_root: Option<PathBuf>,
    host_filter: Option<&str>,
    args: arguments::SshDeploy,
) -> Result<()> {
    let context = ProjectContext::load_project(build_machines, project_root, host_filter, None)
        .await
        .context("Failed to initalize build")?;

    context.run_against_hosts(
        |list| {
            if args.destination.is_some() {
                if list.len() == 1 {
                    Ok(())
                } else {
                    bail!("Host name can only be overriden when deploying to a single host. Use a host filter to limit to a single host.")
                }
            } else {
                Ok(())
            }
        },
        async |host| {
            let hostname = args
                .destination
                .clone()
                .unwrap_or_else(|| format!("root@{}", host));
            context.deploy_ssh(
                host,
                &hostname,
                args.operation,
                !args.no_auto_revert
            ).await
        },
    ).await?;

    Ok(())
}

async fn build_disk_images(
    build_machines: Vec<String>,
    project_root: Option<PathBuf>,
    host_filter: Option<&str>,
    args: arguments::DiskImage,
) -> Result<()> {
    log::info!("Building boot disk images.");

    let context = ProjectContext::load_project(
        build_machines,
        project_root,
        host_filter,
        args.link_path.as_ref().map(|p| p.as_path()),
    )
    .await
    .context("Failed to initalize build")?;

    context
        .run_against_hosts(
            |_hosts| Ok(()),
            async |host| {
                context
                    .run_build(
                        host,
                        &format!(".#nixosConfigurations.{host}.config.system.build.raw"),
                    )
                    .await?;

                Ok(())
            },
        )
        .await?;

    log::info!("Build successful.");

    Ok(())
}

async fn build_iso_installer(
    build_machines: Vec<String>,
    project_root: Option<PathBuf>,
    host_filter: Option<&str>,
    args: arguments::InstallISO,
) -> Result<()> {
    log::info!("Building installer ISO images.");

    let context = ProjectContext::load_project(
        build_machines,
        project_root,
        host_filter,
        args.link_path.as_ref().map(|p| p.as_path()),
    )
    .await
    .context("Failed to initalize build")?;

    context
        .run_against_hosts(
            |_list| Ok(()),
            async |host| {
                context
                    .run_build(
                        host,
                        &format!(".#nixosConfigurations.{host}.config.system.build.installer_iso"),
                    )
                    .await?;

                Ok(())
            },
        )
        .await?;

    log::info!("Build successful.");

    Ok(())
}

async fn install_netboot<'a>(
    build_machines: Vec<String>,
    project_root: Option<PathBuf>,
    host_filter: Option<&str>,
    _args: arguments::InstallNetboot,
) -> Result<()> {
    let context = ProjectContext::load_project(build_machines, project_root, host_filter, None)
        .await
        .context("Failed to initalize build")?;

    context
        .run_against_hosts(
            |_list| Ok(()),
            async |host| {
                let output_directory = context
                    .run_build(
                        host,
                        &format!(
                            ".#nixosConfigurations.{host}.config.system.build.installer_netboot"
                        ),
                    )
                    .await?
                    .canonicalize()
                    .context("Failed to canonicalize path to PXE boot dependencies")?;

                let mut command = Command::new("sudo");

                let kernel = output_directory.join("kernel/bzImage").canonicalize().context("Failed to canonacalize kernel path.")?;
                let initrd = output_directory.join("netbootRamdisk/initrd").canonicalize().context("Failed to canonicalize initrd path")?;
                let root_filesystem = output_directory.join("toplevel/init").canonicalize().context("Failed to canonicalize path to root filesystem")?;

                command.arg("pixiecore");
                command.arg("boot");
                command.arg(kernel);
                command.arg(initrd);
                command.arg("--cmdline");
                command.arg(format!(
                    "init={} loglevel=4",
                    root_filesystem.to_string_lossy()
                ));
                command.arg("--debug");
                command.arg("--dhcp-no-bind");
                command.args(["--port", "64172"]);
                command.args(["--status-port", "64172"]);

                let mut pixiecore = command.spawn().context("Failed to spawn pixiecore")?;
                log::info!("Hosting PXE boot for {host}, please boot that computer now.");
                log::info!("Pixiecore may ask for your password. It needs root privledges to open a UDP socket on port 67.");
                log::info!("Press Ctrl-C to end PXE hosting session and continue.");

                // Wait for ctrl C.
                tokio::signal::ctrl_c()
                    .await
                    .context("Failed to capture Ctrl-C signal")?;

                if let Some(id) = pixiecore.id() {
                    use nix::{unistd::Pid, sys::signal::{self, Signal}};

                    signal::kill(Pid::from_raw(id as i32), Signal::SIGTERM).context("Failed to kill pixiecore")?;
                }
                pixiecore.wait().await.context("Failed to wait for pixiecore to complete")?;
                log::info!("Pixiecore terminated.");

                Ok(())
            },
        )
        .await?;

    Ok(())
}

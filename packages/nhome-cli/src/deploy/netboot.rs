use std::{path::PathBuf, process::Stdio, time::Duration};

use crate::{arguments, ProjectContext};
use anyhow::{Context, Result};
use tokio::process::{Child, Command};

async fn run_pixiecore(context: &ProjectContext, host: &str) -> Result<Child> {
    let output_directory = context
        .run_build(
            host,
            &format!(".#nixosConfigurations.{host}.config.system.build.installer_netboot"),
        )
        .await?
        .canonicalize()
        .context("Failed to canonicalize path to PXE boot dependencies")?;

    let kernel = output_directory
        .join("kernel/bzImage")
        .canonicalize()
        .context("Failed to canonacalize kernel path.")?;
    let initrd = output_directory
        .join("netbootRamdisk/initrd")
        .canonicalize()
        .context("Failed to canonicalize initrd path")?;
    let root_filesystem = output_directory
        .join("toplevel/init")
        .canonicalize()
        .context("Failed to canonicalize path to root filesystem")?;

    let mut command = Command::new("sudo");
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

    Ok(command.spawn().context("Failed to spawn pixiecore")?)
}

async fn upload(context: &ProjectContext, host: &str) -> Result<Child> {
    let top_level = context
        .run_build(
            host,
            &format!(".#nixosConfigurations.{host}.config.system.build.toplevel"),
        )
        .await?
        .canonicalize()
        .context("Failed to canonicalize path to system top-level derivation")?;
    let disko_script = context
            .run_build(
                host,
                &format!(".#nixosConfigurations.{host}.config.system.build.diskoScript"),
            )
            .await.context("Failed to build disko script. Did you remember to include disko module and configuration?")?
            .canonicalize()
            .context("Failed to canonicalize path to disko script")?;

    let mut command = Command::new("nixos-anywhere");
    command.arg("--store-paths");
    command.arg(disko_script);
    command.arg(top_level);
    command.arg(format!("root@{host}"));

    Ok(command
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to spawn nixos-anywhere")?)
}

async fn upload_loop(context: &ProjectContext, host: &str) -> Result<()> {
    loop {
        log::info!("Waiting to start upload");
        tokio::time::sleep(Duration::from_secs(5)).await;

        log::info!("Starting upload");
        let mut upload = upload(context, host)
            .await
            .context("Failed to start upload process")?;

        let result = upload.wait().await;

        match result {
            Ok(exit_status) => {
                if exit_status.success() {
                    log::info!("Upload complete");
                    break Ok(());
                } else {
                    if let Some(exit_code) = exit_status.code() {
                        log::warn!("nixos-anywhere returned non-zero exit status: {exit_code}");
                    } else {
                        log::warn!("nixos-anywhere failed and did not return an exit status");
                    }
                }
            }
            Err(error) => {
                log::warn!("Failed to upload to upload to remote device: {error}");
            }
        }
    }
}

pub async fn install_netboot<'a>(
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


                let mut pixiecore = run_pixiecore(&context, host).await?;

                log::info!("Hosting PXE boot for {host}, please boot that computer now.");
                log::info!("Pixiecore may ask for your password. It needs root privledges to open sockets related to DHCP.");
                log::info!("Press Ctrl-C to end PXE hosting session.");

                let upload = upload_loop(&context, host);

                tokio::select! {
                    result = upload => {
                        result.context("Failed to upload configuration to target")?;
                    }
                    result = pixiecore.wait() => {
                        log::warn!("Pixiecore terminated unexpectedly");
                        result.context("Pixiecore failed")?;
                    }
                    // Wait for ctrl C.
                    result = tokio::signal::ctrl_c() => {
                        result.context("Failed to capture Ctrl-C signal")?;
                        log::info!("Operation aborted by user (Ctrl-C)");
                    }
                }

                // We need to make sure pixiecore properly terminates.
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

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use crate::{arguments, ProjectContext};
use anyhow::{bail, Context, Result};
use tokio::{
    io::{AsyncBufReadExt as _, BufReader},
    process::{Child, Command},
};

async fn run_pixiecore(output_directory: &Path) -> Result<Child> {
    let kernel = {
        let mut kernel = output_directory.join("kernel/bzImage");
        if !kernel.exists() {
            // Sometimes the kernel is not compressed.
            kernel = output_directory.join("kernel/Image");
        }

        let kernel = kernel
            .canonicalize()
            .context("Failed to canonacalize kernel path.")?;

        kernel.to_string_lossy().into_owned()
    };
    let initrd = output_directory
        .join("netbootRamdisk/initrd")
        .canonicalize()
        .context("Failed to canonicalize initrd path")?
        .to_string_lossy()
        .to_string();
    let root_filesystem = output_directory
        .join("toplevel/init")
        .canonicalize()
        .context("Failed to canonicalize path to root filesystem")?
        .to_string_lossy()
        .to_string();

    let sh_input =
        format!("printf 'PIXIECORE START\\n' >&2; pixiecore boot {kernel} {initrd} --cmdline 'init={root_filesystem} loglevel=4' --debug --dhcp-no-bind --port 64172 --status-port 64172");

    let mut command = Command::new("sudo");
    command.arg("sh");
    command.arg("-c");
    command.arg(sh_input);

    log::info!("Pixiecore may ask for your password. It needs root privledges to open sockets related to DHCP.");
    let mut pixiecore = command
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn pixiecore")?;

    // We wait for the start signal before we continue the program.
    let stderr = pixiecore
        .stderr
        .take()
        .context("Failed to capture stderr output of pixiecore")?;
    let mut reader = BufReader::new(stderr);

    let mut line = String::new();
    loop {
        let length = reader
            .read_line(&mut line)
            .await
            .context("Failed to read stderr line from pixiecore")?;

        if length == 0 || line == "PIXIECORE START\n" {
            break;
        }

        line.clear();
    }

    // The sudo call tends to leave the terminal in a bad state, so we're
    // going to reset it to a sane state.
    Command::new("stty").arg("sane").status().await.ok();
    Ok(pixiecore)
}

async fn handle_pixiecore_termination(mut pixiecore: Child) -> Result<()> {
    // We need to make sure pixiecore properly terminates.
    if let Some(id) = pixiecore.id() {
        use nix::{
            sys::signal::{self, Signal},
            unistd::Pid,
        };

        signal::kill(Pid::from_raw(id as i32), Signal::SIGTERM)
            .context("Failed to kill pixiecore")?;
    }
    pixiecore
        .wait()
        .await
        .context("Failed to wait for pixiecore to complete")?;
    log::info!("Pixiecore terminated.");

    Ok(())
}

async fn solo_run_pixiecore(pxe_boot_directory: &Path) -> Result<()> {
    let mut pixiecore = run_pixiecore(&pxe_boot_directory).await?;

    tokio::select! {
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

    handle_pixiecore_termination(pixiecore).await?;

    Ok(())
}

async fn upload(top_level: &Path, disko_script: &Path, host: &str) -> Result<Child> {
    let mut command = Command::new("nixos-anywhere");
    command.arg("--store-paths");
    command.arg(disko_script);
    command.arg(top_level);
    command.arg(format!("root@{host}"));
    command.kill_on_drop(true);

    Ok(command.spawn().context("Failed to spawn nixos-anywhere")?)
}

async fn solo_upload(top_level: &Path, disko_script: &Path, host: &str) -> Result<()> {
    log::info!("Starting upload");
    let mut upload = upload(top_level, disko_script, host)
        .await
        .context("Failed to start upload process")?;

    let result = upload.wait().await;

    match result {
        Ok(exit_status) => {
            if exit_status.success() {
                log::info!("Upload complete");
                Ok(())
            } else {
                if let Some(exit_code) = exit_status.code() {
                    bail!("nixos-anywhere returned non-zero exit status: {exit_code}");
                } else {
                    bail!("nixos-anywhere failed and did not return an exit status");
                }
            }
        }
        Err(error) => {
            bail!("Failed to upload to upload to remote device: {error}");
        }
    }
}

async fn upload_loop(top_level: &Path, disko_script: &Path, host: &str) -> Result<()> {
    loop {
        log::info!("Waiting to start upload");
        tokio::time::sleep(Duration::from_secs(5)).await;

        log::info!("Starting upload");
        let mut upload = upload(top_level, disko_script, host)
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

async fn run_both(
    host: &str,
    pxe_boot_directory: &Path,
    top_level: &Path,
    disko_script: &Path,
) -> Result<()> {
    let mut pixiecore = run_pixiecore(&pxe_boot_directory).await?;

    log::info!("Hosting PXE boot for {host}, please boot that computer now.");
    log::info!("Press Ctrl-C to end PXE hosting session.");

    let upload = upload_loop(&top_level, &disko_script, host);

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

    handle_pixiecore_termination(pixiecore).await?;

    Ok(())
}

pub async fn install_netboot<'a>(
    build_machines: Vec<String>,
    project_root: Option<PathBuf>,
    host_filter: Option<&str>,
    args: arguments::InstallNetboot,
) -> Result<()> {
    let context = ProjectContext::load_project(build_machines, project_root, host_filter, None)
        .await
        .context("Failed to initalize build")?;

    context
        .run_against_hosts(|_list| Ok(()), async |host| {
            let pxe_boot_directory = context
                .run_build(
                    host,
                    &format!(".#nixosConfigurations.{host}.config.system.build.installer_netboot"),
                )
                .await?
                .canonicalize()
                .context("Failed to canonicalize path to PXE boot dependencies")?;
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

            match args.steps {
                arguments::netboot::Steps::Both(_) => run_both(host, &pxe_boot_directory, &top_level, &disko_script).await?,
                arguments::netboot::Steps::Boot(_) => solo_run_pixiecore(&pxe_boot_directory).await?,
                arguments::netboot::Steps::Install(_) => solo_upload(&top_level, &disko_script, host).await?,
            }

            Ok(())
        })
        .await?;

    Ok(())
}

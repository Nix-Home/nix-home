use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use regex::Regex;
use tokio::process::Command;

use crate::arguments;
use crate::nix_eval_hosts;

const PROBE_SCRIPT: &str = include_str!("probe.sh");

/// Overall time budget for probing a single host. On top of ssh's own
/// ConnectTimeout, this bounds how long a dead host can hold up the run.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq)]
enum BootTarget {
    /// Store path of the system the bootloader will boot next.
    System(String),
    /// No supported bootloader configuration could be found on the host.
    None,
}

#[derive(Debug, Clone)]
struct Probe {
    state: String,
    current: String,
    boot: BootTarget,
    container: Option<String>,
    current_gen: Option<u32>,
    boot_gen: Option<u32>,
}

#[derive(Debug, Clone)]
enum HostProbe {
    Probed(Probe),
    Unreachable(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Verdict {
    InSync,
    Drifted,
    NoBootloader,
    UnsupportedBootloader,
    Undetermined,
}

fn verdict(probe: &Probe) -> Verdict {
    let boot_path = match &probe.boot {
        BootTarget::None => {
            return if probe.container.is_some() {
                Verdict::NoBootloader
            } else {
                Verdict::UnsupportedBootloader
            };
        }
        BootTarget::System(path) => path,
    };

    if probe.current == "unknown" || boot_path == "unknown" {
        return Verdict::Undetermined;
    }

    if probe.current == *boot_path {
        Verdict::InSync
    } else {
        Verdict::Drifted
    }
}

fn parse_probe_output(output: &str) -> Result<Probe> {
    let mut fields: HashMap<String, String> = HashMap::new();
    for line in output.lines() {
        if let Some((key, value)) = line.split_once(' ') {
            if !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                fields.insert(key.to_string(), value.to_string());
            }
        }
    }

    let state = fields.get("STATE").context("Probe output is missing STATE")?;
    let current = fields.get("CURRENT").context("Probe output is missing CURRENT")?;
    let boot_raw = fields.get("BOOT").context("Probe output is missing BOOT")?;

    let boot = match boot_raw.as_str() {
        "none" => BootTarget::None,
        path => BootTarget::System(path.to_string()),
    };

    Ok(Probe {
        state: state.clone(),
        current: current.clone(),
        boot,
        // `systemd-detect-virt -c` prints "none" when not in a container.
        container: fields
            .get("CONTAINER")
            .filter(|v| !v.is_empty() && v.as_str() != "none")
            .cloned(),
        current_gen: fields.get("CURRENT_GEN").and_then(|v| v.parse().ok()),
        boot_gen: fields.get("BOOT_GEN").and_then(|v| v.parse().ok()),
    })
}

/// Runs the probe script on a host over ssh and returns its raw output.
pub(crate) async fn run_probe(ssh_config_path: &Path, host: &str, script: &str) -> Result<String> {
    let mut command = Command::new("ssh");
    command
        .arg("-F")
        .arg(ssh_config_path)
        .arg("-o ConnectTimeout=10")
        .arg("-o BatchMode=yes")
        .arg(host)
        .arg(script);

    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .await
        .with_context(|| format!("Failed to run ssh against {host}"))?;

    if !output.status.success() {
        bail!("ssh to {host} failed with status {}", output.status);
    }

    let output = String::from_utf8(output.stdout)
        .with_context(|| format!("Probe output from {host} is not utf8 encoded text"))?;
    Ok(output)
}

async fn probe_host(ssh_config_path: String, host: String) -> HostProbe {
    let result = tokio::time::timeout(
        PROBE_TIMEOUT,
        run_probe(Path::new(&ssh_config_path), &host, PROBE_SCRIPT),
    )
    .await;

    match result {
        Err(_elapsed) => {
            HostProbe::Unreachable(format!("timed out after {}s", PROBE_TIMEOUT.as_secs()))
        }
        Ok(Ok(output)) => match parse_probe_output(&output) {
            Ok(probe) => HostProbe::Probed(probe),
            Err(error) => {
                log::warn!("Could not parse probe output from {host}: {error:?}");
                log::debug!("Raw probe output:\n{output}");
                HostProbe::Unreachable(format!("could not parse probe output: {error}"))
            }
        },
        Ok(Err(error)) => HostProbe::Unreachable(error.to_string()),
    }
}

/// Turns a store path into a short human readable label, e.g.
/// `/nix/store/abc…-nixos-system-carlcore-26.05.x` becomes `carlcore 26.05.x`.
fn system_label(path: &str) -> String {
    if path.is_empty() || path == "unknown" {
        return "-".to_string();
    }

    let Some(rest) = path.strip_prefix("/nix/store/") else {
        return path.to_string();
    };

    // `rest` is `<hash>-<name>`, optionally followed by `/specialisation/<s>`.
    let (base, specialisation) = match rest.split_once("/specialisation/") {
        Some((base, spec)) => (base, Some(spec)),
        None => (rest, None),
    };
    let name = base.split_once('-').map(|(_, name)| name).unwrap_or(base);
    let name = name.strip_prefix("nixos-system-").unwrap_or(name);

    // Split "<hostname>-<version>" so the label reads "hostname version".
    let re = Regex::new(r"^(.*)-(\d.*)$").expect("static regex is valid");
    let name = if let Some(caps) = re.captures(name) {
        format!(
            "{} {}",
            caps.get(1).expect("group 1 must match").as_str(),
            caps.get(2).expect("group 2 must match").as_str()
        )
    } else {
        name.to_string()
    };

    match specialisation {
        Some(spec) => format!("{name} ({spec})"),
        None => name,
    }
}

fn generation_cell(path: &str, gen: Option<u32>) -> String {
    if path.is_empty() || path == "unknown" {
        return "-".to_string();
    }

    let label = system_label(path);
    match gen {
        Some(n) => format!("gen{n} ({label})"),
        None => label,
    }
}

fn verdict_text(verdict: &Verdict) -> &'static str {
    match verdict {
        Verdict::InSync => "in sync",
        Verdict::Drifted => "OUT OF SYNC",
        Verdict::NoBootloader => "no bootloader (container)",
        Verdict::UnsupportedBootloader => "unsupported bootloader",
        Verdict::Undetermined => "undetermined",
    }
}

/// Returns the process exit code: 0 if all hosts are healthy, 1 if some hosts
/// are unhealthy but at least one host could be probed, 3 if no host status
/// could be obtained at all.
pub async fn status(args: arguments::Status) -> i32 {
    match run_status(args).await {
        Ok(code) => code,
        Err(error) => {
            log::error!("Fatal error: {error:?}");
            3
        }
    }
}

async fn run_status(args: arguments::Status) -> Result<i32> {
    let project_root = match args.project_root {
        Some(root) => root,
        None => std::env::current_dir().context("Failed to get current directory")?,
    };
    log::info!("Project root: {project_root:?}");

    // Read-only by design: do not create files and do not use ProjectContext,
    // which would delete an existing `<project_root>/result` output.
    let ssh_config_path = project_root.join("ssh_config");
    if !ssh_config_path.exists() {
        bail!("Project is missing `ssh_config` file.");
    }

    let host_filter = match &args.hosts {
        Some(filter) => {
            log::info!("Host filter: '{filter}'");
            Some(Regex::new(filter).context("Failed to compile regex expression for host filter")?)
        }
        None => None,
    };

    let mut hosts = nix_eval_hosts(&project_root)
        .await
        .context("Failed to get list of hosts from flake.nix")?;
    if let Some(filter) = &host_filter {
        hosts.retain(|host| filter.is_match(host));
    }

    if hosts.is_empty() {
        bail!("No hosts matched the given filter.");
    }

    log::info!(
        "Probing {} host(s): {}",
        hosts.len(),
        hosts.join(", ")
    );

    // Probe all hosts in parallel; a slow or dead host must not block the rest.
    let ssh_config_str = ssh_config_path.to_string_lossy().into_owned();
    let mut handles = Vec::new();
    for (index, host) in hosts.iter().enumerate() {
        let ssh_config_path = ssh_config_str.clone();
        let host = host.clone();
        handles.push(tokio::spawn(async move {
            (index, probe_host(ssh_config_path, host).await)
        }));
    }

    let mut results: Vec<Option<HostProbe>> = vec![None; hosts.len()];
    for handle in handles {
        match handle.await {
            Ok((index, probe)) => results[index] = Some(probe),
            Err(error) => log::warn!("A probe task panicked: {error:?}"),
        }
    }

    // Build the display rows: (host, systemd state, running, boot, verdict).
    let mut rows: Vec<[String; 5]> = Vec::with_capacity(hosts.len());
    for (host, result) in hosts.iter().zip(results.into_iter()) {
        match result {
            Some(HostProbe::Probed(probe)) => {
                let verdict = verdict(&probe);
                rows.push([
                    host.clone(),
                    probe.state.clone(),
                    generation_cell(&probe.current, probe.current_gen),
                    match &probe.boot {
                        BootTarget::System(path) => generation_cell(path, probe.boot_gen),
                        BootTarget::None => "-".to_string(),
                    },
                    verdict_text(&verdict).to_string(),
                ]);
            }
            Some(HostProbe::Unreachable(error)) => rows.push([
                host.clone(),
                format!("unreachable ({error})"),
                "-".to_string(),
                "-".to_string(),
                "unreachable".to_string(),
            ]),
            None => rows.push([
                host.clone(),
                "unreachable".to_string(),
                "-".to_string(),
                "-".to_string(),
                "probe failed".to_string(),
            ]),
        }
    }

    // Print the table with aligned columns.
    let header = ["host", "systemd", "running", "boot", "verdict"];
    let widths: Vec<usize> = (0..5)
        .map(|col| {
            header[col]
                .len()
                .max(rows.iter().map(|row| row[col].len()).max().unwrap_or(0))
        })
        .collect();

    println!(
        "{}",
        header
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{h:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in &rows {
        println!(
            "{}",
            row.iter()
                .enumerate()
                .map(|(i, cell)| format!("{cell:<w$}", w = widths[i]))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }

    let total = rows.len();
    let healthy = |row: &&[String; 5]| {
        matches!(
            row[4].as_str(),
            "in sync" | "no bootloader (container)" | "unsupported bootloader"
        )
    };
    let unhealthy = rows.iter().filter(|row| !healthy(row)).count();
    let unsupported = rows
        .iter()
        .filter(|row| row[4] == "unsupported bootloader")
        .count();

    if unhealthy > 0 {
        println!("{unhealthy} of {total} host(s) need attention.");
    } else {
        println!("All {total} host(s) healthy.");
    }
    if unsupported > 0 {
        log::warn!(
            "{unsupported} host(s) have bootloaders that are not supported yet; their boot default could not be checked."
        );
    }

    let probed = rows
        .iter()
        .filter(|row| !matches!(row[4].as_str(), "unreachable" | "probe failed"))
        .count();
    if probed == 0 {
        Ok(3)
    } else if unhealthy > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_output(boot: &str, container: &str) -> String {
        format!(
            "STATE running\n\
             CURRENT /nix/store/aaaa-nixos-system-test-26.05.abc\n\
             BOOT {boot}\n\
             CONTAINER {container}\n\
             CURRENT_GEN 14\n\
             BOOT_GEN 14\n"
        )
    }

    #[test]
    fn parse_probe_output_parses_full_output() {
        let probe = parse_probe_output(&sample_output(
            "/nix/store/aaaa-nixos-system-test-26.05.abc",
            "",
        ))
        .expect("expected sample output to parse");

        assert_eq!(probe.state, "running");
        assert_eq!(probe.current, "/nix/store/aaaa-nixos-system-test-26.05.abc");
        assert_eq!(
            probe.boot,
            BootTarget::System("/nix/store/aaaa-nixos-system-test-26.05.abc".to_string())
        );
        assert_eq!(probe.container, None);
        assert_eq!(probe.current_gen, Some(14));
        assert_eq!(probe.boot_gen, Some(14));
    }

    #[test]
    fn parse_probe_output_accepts_missing_optional_fields() {
        let output = "STATE degraded\nCURRENT /nix/store/aaaa-nixos-system-test-x\nBOOT none\n";
        let probe = parse_probe_output(output).expect("expected minimal output to parse");

        assert_eq!(probe.state, "degraded");
        assert_eq!(probe.boot, BootTarget::None);
        assert_eq!(probe.container, None);
        assert_eq!(probe.current_gen, None);
        assert_eq!(probe.boot_gen, None);
    }

    #[test]
    fn parse_probe_output_rejects_missing_required_fields() {
        assert!(parse_probe_output("CURRENT /nix/store/x\nBOOT none\n").is_err());
        assert!(parse_probe_output("STATE running\nBOOT none\n").is_err());
        assert!(parse_probe_output("STATE running\nCURRENT /nix/store/x\n").is_err());
        assert!(parse_probe_output("").is_err());
    }

    #[test]
    fn verdict_reports_in_sync_when_paths_match() {
        let probe = parse_probe_output(&sample_output(
            "/nix/store/aaaa-nixos-system-test-26.05.abc",
            "",
        ))
        .unwrap();
        assert_eq!(verdict(&probe), Verdict::InSync);
    }

    #[test]
    fn verdict_reports_drifted_when_paths_differ() {
        let output = sample_output("/nix/store/bbbb-nixos-system-test-old", "");
        let probe = parse_probe_output(&output).unwrap();
        assert_eq!(verdict(&probe), Verdict::Drifted);
    }

    #[test]
    fn verdict_reports_no_bootloader_for_containers() {
        let output = sample_output("none", "lxc");
        let probe = parse_probe_output(&output).unwrap();
        assert_eq!(verdict(&probe), Verdict::NoBootloader);
        assert_eq!(probe.container, Some("lxc".to_string()));
    }

    #[test]
    fn parse_probe_output_treats_none_container_as_absent() {
        let output = "STATE running\nCURRENT /nix/store/aaaa-nixos-system-test-x\nBOOT none\nCONTAINER none\n";
        let probe = parse_probe_output(output).expect("expected output to parse");
        assert_eq!(probe.container, None);
        assert_eq!(verdict(&probe), Verdict::UnsupportedBootloader);
    }

    #[test]
    fn verdict_reports_unsupported_bootloader_without_loader() {
        let output = sample_output("none", "");
        let probe = parse_probe_output(&output).unwrap();
        assert_eq!(verdict(&probe), Verdict::UnsupportedBootloader);
    }

    #[test]
    fn verdict_reports_undetermined_when_current_system_is_unknown() {
        let output = "STATE running\nCURRENT unknown\nBOOT /nix/store/aaaa-nixos-system-test-26.05.abc\n";
        let probe = parse_probe_output(output).unwrap();
        assert_eq!(verdict(&probe), Verdict::Undetermined);
    }

    #[test]
    fn system_label_renders_names_and_specialisations() {
        assert_eq!(
            system_label("/nix/store/aaaa-nixos-system-carlcore-26.05.abc"),
            "carlcore 26.05.abc"
        );
        assert_eq!(
            system_label(
                "/nix/store/aaaa-nixos-system-carlcore-26.05.abc/specialisation/wired"
            ),
            "carlcore 26.05.abc (wired)"
        );
        assert_eq!(system_label("unknown"), "-");
        assert_eq!(system_label(""), "-");
    }

    #[test]
    fn generation_cell_shows_generation_number_when_available() {
        assert_eq!(
            generation_cell("/nix/store/aaaa-nixos-system-carlcore-26.05.abc", Some(14)),
            "gen14 (carlcore 26.05.abc)"
        );
        assert_eq!(
            generation_cell("/nix/store/aaaa-nixos-system-carlcore-26.05.abc", None),
            "carlcore 26.05.abc"
        );
        assert_eq!(generation_cell("unknown", Some(14)), "-");
    }

    /// Runs the probe script locally through `sh`. Only meaningful on a NixOS
    /// system, so this test skips itself elsewhere. Note that reading the ESP
    /// typically requires root, so without root the BOOT value will be none;
    /// the assertions here only check that the script runs and emits its keys.
    #[test]
    fn probe_script_runs_locally() {
        if !Path::new("/run/current-system").exists() {
            eprintln!("skipping: /run/current-system not found (not a NixOS system?)");
            return;
        }

        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/status/probe.sh");
        let output = std::process::Command::new("sh")
            .arg(&script)
            .output()
            .expect("failed to run probe script");

        assert!(
            output.status.success(),
            "probe script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        for key in ["STATE", "CURRENT", "BOOT", "CONTAINER"] {
            assert!(
                stdout.lines().any(|line| line.starts_with(&format!("{key} "))),
                "missing {key} line in:\n{stdout}"
            );
        }

        let current = stdout
            .lines()
            .find(|line| line.starts_with("CURRENT "))
            .unwrap();
        assert!(
            current.contains("/nix/store/") || current.ends_with("unknown"),
            "unexpected CURRENT line: {current}"
        );
    }

    /// Probes real hosts over ssh using the exact same code path as the
    /// `status` subcommand. Skipped unless both NHOME_TEST_SSH_CONFIG and
    /// NHOME_TEST_HOSTS are set, e.g.:
    ///
    ///   NHOME_TEST_SSH_CONFIG=$HOME/nix-configuration/ssh_config \
    ///   NHOME_TEST_HOSTS=dev-vm,carlcore,inference-vm \
    ///   cargo test status_probe_over_ssh
    #[tokio::test]
    async fn status_probe_over_ssh() {
        let (cfg_env, hosts_env) = match (
            std::env::var("NHOME_TEST_SSH_CONFIG"),
            std::env::var("NHOME_TEST_HOSTS"),
        ) {
            (Ok(cfg), Ok(hosts)) => (cfg, hosts),
            _ => {
                eprintln!(
                    "skipping: set NHOME_TEST_SSH_CONFIG and NHOME_TEST_HOSTS to enable"
                );
                return;
            }
        };

        let cfg = std::path::PathBuf::from(&cfg_env);
        if !cfg.exists() {
            panic!("NHOME_TEST_SSH_CONFIG does not exist: {cfg:?}");
        }

        for host in hosts_env.split(',') {
            let host = host.trim();
            if host.is_empty() {
                continue;
            }

            let output = match run_probe(&cfg, host, PROBE_SCRIPT).await {
                Ok(output) => output,
                Err(error) => panic!(
                    "probe of {host} failed: {error:?}\n\
                     Try manually:\n  ssh -F {} {host} \"$(cat src/status/probe.sh)\"",
                    cfg.display()
                ),
            };

            let probe = parse_probe_output(&output).unwrap_or_else(|error| {
                panic!("could not parse probe output for {host}: {error:?}\n---\n{output}")
            });

            let verdict = verdict(&probe);
            println!(
                "{host}: state={} current={} boot={:?} verdict={verdict:?}",
                probe.state, probe.current, probe.boot
            );

            assert!(
                matches!(verdict, Verdict::InSync | Verdict::NoBootloader),
                "expected {host} to be in sync or a container, got {verdict:?}"
            );
        }
    }
}

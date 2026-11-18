//! Installs Chocolatey from its official bootstrap script, the same source the PowerShell original
//! uses. The script is downloaded to a temporary file and run in a new `powershell.exe`, so its
//! output and exit code stay separate from this process.
//!
//! The download and the process spawn are closures passed to [`install_with`], so the decisions
//! are testable without touching the network or installing anything.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::Availability;
use super::run::{Manager, Outcome, Report, last_lines, run_command};
use crate::net;

/// The bootstrap script that the official install command downloads.
pub const INSTALL_SCRIPT_URL: &str = "https://community.chocolatey.org/install.ps1";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Shown as the package name in the log, since the script is not a package.
const SCRIPT_LABEL: &str = "install.ps1";

/// Windows PowerShell 5.1 can negotiate a TLS version the Chocolatey servers refuse. The official
/// one-liner sets TLS 1.2 (flag 3072) before running the script, and so does this.
const ENABLE_TLS12: &str =
    "[System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072";

/// The `-Command` text that enables TLS 1.2 and runs the bootstrap script at `script`.
pub fn powershell_command(script: &Path) -> String {
    let quoted = script.to_string_lossy().replace('\'', "''");
    format!("{ENABLE_TLS12}; & '{quoted}'")
}

/// Installs Chocolatey when it is missing. `download` fetches `INSTALL_SCRIPT_URL` to `script`,
/// `run` executes a program and returns its exit code and output (or `None` if it did not start),
/// and `is_installed` reports whether `choco` is now on the search path.
pub fn install_with(
    script: &Path,
    is_installed: impl Fn() -> bool,
    download: impl FnOnce(&str, &Path) -> Result<(), String>,
    run: impl FnOnce(&str, &[&str]) -> Option<(i32, String)>,
) -> Report {
    if is_installed() {
        return report(Outcome::Skipped, -1, "Chocolatey is already installed.".into(), Vec::new());
    }
    if let Err(error) = download(INSTALL_SCRIPT_URL, script) {
        return report(
            Outcome::Failed,
            -1,
            format!("could not download {INSTALL_SCRIPT_URL}: {error}"),
            Vec::new(),
        );
    }

    let command = powershell_command(script);
    let args = ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command.as_str()];
    let Some((code, output)) = run("powershell.exe", &args) else {
        return report(
            Outcome::Failed,
            -1,
            "could not start powershell.exe to run the Chocolatey installer".into(),
            Vec::new(),
        );
    };
    if code != 0 {
        return report(
            Outcome::Failed,
            code,
            format!("the Chocolatey installer exited with code {code}."),
            last_lines(&output),
        );
    }
    if is_installed() {
        return report(Outcome::Succeeded, 0, "Chocolatey installed.".into(), Vec::new());
    }
    report(
        Outcome::Failed,
        code,
        "Chocolatey was installed but choco is still not on PATH.".into(),
        last_lines(&output),
    )
}

/// Installs Chocolatey on this machine, using the real download and process spawn.
pub fn install() -> Report {
    let script = script_path();
    let report = install_with(
        &script,
        || Availability::detect().choco,
        |url, dest| net::download(url, dest, DOWNLOAD_TIMEOUT),
        run_command,
    );
    // The script is only needed for this run; a leftover copy would be stale next time.
    let _ = fs::remove_file(&script);
    report
}

fn script_path() -> PathBuf {
    std::env::temp_dir().join("winutil-rs").join("install-chocolatey.ps1")
}

fn report(outcome: Outcome, exit_code: i32, detail: String, tail: Vec<String>) -> Report {
    Report {
        package: SCRIPT_LABEL.to_owned(),
        manager: Manager::Choco,
        exit_code,
        outcome,
        detail,
        tail,
    }
}

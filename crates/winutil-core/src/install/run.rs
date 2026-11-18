//! Runs install commands and reports each package's outcome the way the original does.
//!
//! Exit codes are classified by pure functions so the mapping is testable without installing
//! anything. Process spawning is isolated in [`run_command`], and orchestration takes the runner as
//! a parameter so tests can substitute it.

use std::process::Command;

use super::msi::{self, MsiHost, system::SystemMsi};
use super::{Plan, choco_install_args, winget_install_args, winget_source};

/// How many trailing output lines a failed package keeps for the log.
const TAIL_LINES: usize = 15;

/// Whether a package ended up installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Winget,
    Choco,
    /// The direct PowerShell 7 MSI upgrade, which is not a package manager.
    Msi,
}

/// The result of installing one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Bare package ID, without the `msstore:` prefix.
    pub package: String,
    pub manager: Manager,
    /// Process exit code, or -1 when the command never started.
    pub exit_code: i32,
    pub outcome: Outcome,
    pub detail: String,
    /// Last lines of command output, kept only for failures.
    pub tail: Vec<String>,
}

/// Installs every package in `plan` using the real `winget` and `choco` commands, and the direct
/// MSI upgrade for PowerShell 7. WinGet runs first, then Chocolatey, matching the original order.
/// `on_report` runs after each package so a caller can show progress.
pub fn run_plan(plan: &Plan, on_report: impl FnMut(&Report)) -> Vec<Report> {
    run_plan_with(plan, run_command, &mut SystemMsi, on_report)
}

/// Like [`run_plan`], with the process runner and MSI host supplied by the caller. The runner
/// receives the program and its arguments, and returns the exit code and combined output, or
/// `None` when the program could not be started.
pub fn run_plan_with(
    plan: &Plan,
    mut run: impl FnMut(&str, &[&str]) -> Option<(i32, String)>,
    host: &mut impl MsiHost,
    mut on_report: impl FnMut(&Report),
) -> Vec<Report> {
    let mut reports = Vec::with_capacity(plan.winget.len() + plan.choco.len());
    let mut finish = |report: Report| {
        on_report(&report);
        reports.push(report);
    };

    for raw in &plan.winget {
        let (source, id) = winget_source(raw);
        if id.eq_ignore_ascii_case(msi::PACKAGE_ID) {
            if let Some(report) = msi::update(host) {
                finish(report);
                continue;
            }
        }
        let args = winget_install_args(source, id);
        let report = make_report(
            Manager::Winget,
            id,
            run("winget", &args),
            classify_winget,
            "WinGet did not start",
        );
        finish(report);
    }

    for id in &plan.choco {
        let args = choco_install_args(id);
        let report = make_report(
            Manager::Choco,
            id,
            run("choco", &args),
            classify_choco,
            "Chocolatey command did not start",
        );
        finish(report);
    }

    reports
}

/// Maps a WinGet exit code to an outcome and the detail text the original logs.
pub fn classify_winget(exit_code: i32) -> (Outcome, String) {
    // WinGet refuses to act on a package installed for another user while elevated.
    const ADMIN_CONTEXT_PROHIBITED: i32 = -1978335107;

    match exit_code {
        0 => (Outcome::Succeeded, "exit code 0".to_owned()),
        3010 | -1978334967 => (
            Outcome::Succeeded,
            "installed, a restart is needed to finish".to_owned(),
        ),
        1641 | -1978334965 => (
            Outcome::Succeeded,
            "installed, the installer started a restart".to_owned(),
        ),
        -1978335135 => (Outcome::Skipped, "already installed".to_owned()),
        -1978335189 => (Outcome::Skipped, "no applicable update".to_owned()),
        ADMIN_CONTEXT_PROHIBITED => (
            Outcome::Skipped,
            "already installed for the current user; elevated WinUtil cannot update it".to_owned(),
        ),
        code => (
            Outcome::Failed,
            format!(
                "WinGet reported 0x{code:08X}. See https://learn.microsoft.com/windows/package-manager/winget/returnCodes"
            ),
        ),
    }
}

/// Maps a Chocolatey exit code to an outcome and the detail text the original logs.
pub fn classify_choco(exit_code: i32) -> (Outcome, String) {
    match exit_code {
        0 => (Outcome::Succeeded, "exit code 0".to_owned()),
        1641 => (
            Outcome::Succeeded,
            "installed, the installer started a restart".to_owned(),
        ),
        3010 => (
            Outcome::Succeeded,
            "installed, a restart is needed to finish".to_owned(),
        ),
        2 => (Outcome::Skipped, "nothing to do".to_owned()),
        code => (Outcome::Failed, format!("exit code {code}")),
    }
}

fn make_report(
    manager: Manager,
    package: &str,
    run: Option<(i32, String)>,
    classify: fn(i32) -> (Outcome, String),
    not_started: &str,
) -> Report {
    let (exit_code, outcome, detail, output) = match run {
        Some((code, output)) => {
            let (outcome, detail) = classify(code);
            (code, outcome, detail, output)
        }
        None => (-1, Outcome::Failed, not_started.to_owned(), String::new()),
    };
    let tail = if outcome == Outcome::Failed {
        last_lines(&output)
    } else {
        Vec::new()
    };
    Report {
        package: package.to_owned(),
        manager,
        exit_code,
        outcome,
        detail,
        tail,
    }
}

fn last_lines(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines[start..].iter().map(|line| (*line).to_owned()).collect()
}

/// Runs a program to completion and returns its exit code and combined stdout and stderr.
/// Stdin is not inherited, so a prompt cannot block the run. Returns `None` if the program
/// cannot be started.
pub fn run_command(program: &str, args: &[&str]) -> Option<(i32, String)> {
    let output = Command::new(program).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Some((output.status.code().unwrap_or(-1), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winget_success_and_reboot_codes_succeed() {
        assert_eq!(classify_winget(0), (Outcome::Succeeded, "exit code 0".to_owned()));
        assert_eq!(classify_winget(3010).0, Outcome::Succeeded);
        assert_eq!(classify_winget(-1978334967).0, Outcome::Succeeded);
        assert_eq!(classify_winget(1641).0, Outcome::Succeeded);
        assert_eq!(classify_winget(-1978334965).0, Outcome::Succeeded);
    }

    #[test]
    fn winget_nothing_to_do_and_admin_context_are_skipped() {
        assert_eq!(classify_winget(-1978335135), (Outcome::Skipped, "already installed".to_owned()));
        assert_eq!(classify_winget(-1978335189), (Outcome::Skipped, "no applicable update".to_owned()));
        assert_eq!(classify_winget(-1978335107).0, Outcome::Skipped);
    }

    #[test]
    fn winget_other_codes_fail_with_hex_code() {
        let (outcome, detail) = classify_winget(-2147024891);
        assert_eq!(outcome, Outcome::Failed);
        assert!(detail.starts_with("WinGet reported 0x80070005."), "{detail}");
    }

    #[test]
    fn choco_codes_map_to_outcomes() {
        assert_eq!(classify_choco(0).0, Outcome::Succeeded);
        assert_eq!(classify_choco(1641).0, Outcome::Succeeded);
        assert_eq!(classify_choco(3010).0, Outcome::Succeeded);
        assert_eq!(classify_choco(2), (Outcome::Skipped, "nothing to do".to_owned()));
        assert_eq!(classify_choco(1), (Outcome::Failed, "exit code 1".to_owned()));
    }

    #[test]
    fn missing_program_is_failed_not_started() {
        let report = make_report(Manager::Choco, "git", None, classify_choco, "not started");
        assert_eq!(report.exit_code, -1);
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.detail, "not started");
        assert!(report.tail.is_empty());
    }

    #[test]
    fn failure_keeps_last_nonblank_lines_only() {
        let output = (1..=20).map(|n| format!("line {n}\n\n")).collect::<String>();
        let report = make_report(Manager::Choco, "git", Some((1, output)), classify_choco, "x");
        assert_eq!(report.tail.len(), TAIL_LINES);
        assert_eq!(report.tail.first().map(String::as_str), Some("line 6"));
        assert_eq!(report.tail.last().map(String::as_str), Some("line 20"));
    }

    #[test]
    fn success_drops_output() {
        let report = make_report(Manager::Choco, "git", Some((0, "ok".to_owned())), classify_choco, "x");
        assert!(report.tail.is_empty());
    }

    #[test]
    fn run_command_returns_exit_code_and_output() {
        let (code, output) = run_command("cmd", &["/C", "echo hello & exit 5"]).unwrap();
        assert_eq!(code, 5);
        assert!(output.contains("hello"), "{output:?}");
    }

    #[test]
    fn run_command_returns_none_for_missing_program() {
        assert_eq!(run_command("winutil-rs-no-such-program", &[]), None);
    }
}

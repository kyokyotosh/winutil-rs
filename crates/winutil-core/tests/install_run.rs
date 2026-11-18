//! Install orchestration checked with a fake process runner and MSI host, so no software is
//! installed and the registry is never read.

use std::path::{Path, PathBuf};

use winutil_core::install::Plan;
use winutil_core::install::msi::{MsiHost, Release};
use winutil_core::install::run::{Manager, Outcome, run_plan_with};

fn plan(winget: &[&str], choco: &[&str]) -> Plan {
    Plan {
        winget: winget.iter().map(|id| (*id).to_owned()).collect(),
        choco: choco.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// A host for plans without PowerShell. Any call into the MSI path is a test failure.
struct NoMsi;

impl MsiHost for NoMsi {
    fn msi_installs(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn installed_version(&mut self, _: &Path) -> Result<String, String> {
        unreachable!("MSI path not expected")
    }
    fn architecture(&mut self, _: &Path) -> Result<String, String> {
        unreachable!("MSI path not expected")
    }
    fn upgrade_allowed(&mut self) -> bool {
        unreachable!("MSI path not expected")
    }
    fn latest_release(&mut self) -> Result<Release, String> {
        unreachable!("MSI path not expected")
    }
    fn scratch_path(&mut self, _: &str) -> PathBuf {
        unreachable!("MSI path not expected")
    }
    fn download(&mut self, _: &str, _: &Path) -> Result<(), String> {
        unreachable!("MSI path not expected")
    }
    fn verify(&mut self, _: &Path, _: Option<&str>) -> Result<(), String> {
        unreachable!("MSI path not expected")
    }
    fn install(&mut self, _: &Path) -> Result<i32, String> {
        unreachable!("MSI path not expected")
    }
    fn install_msix(&mut self, _: &Path) -> Result<(), String> {
        unreachable!("MSI path not expected")
    }
    fn remove(&mut self, _: &Path) {
        unreachable!("MSI path not expected")
    }
}

/// A host reporting two PowerShell 7 MSI installs, which the original refuses to touch.
struct TwoMsiInstalls;

impl MsiHost for TwoMsiInstalls {
    fn msi_installs(&mut self) -> Vec<PathBuf> {
        vec![PathBuf::from(r"C:\Program Files\PowerShell\7"), PathBuf::from(r"D:\PowerShell\7")]
    }
    fn installed_version(&mut self, _: &Path) -> Result<String, String> {
        unreachable!("two installs are rejected before any probe")
    }
    fn architecture(&mut self, _: &Path) -> Result<String, String> {
        unreachable!("two installs are rejected before any probe")
    }
    fn upgrade_allowed(&mut self) -> bool {
        unreachable!("two installs are rejected before any probe")
    }
    fn latest_release(&mut self) -> Result<Release, String> {
        unreachable!("two installs are rejected before any probe")
    }
    fn scratch_path(&mut self, _: &str) -> PathBuf {
        unreachable!("two installs are rejected before any probe")
    }
    fn download(&mut self, _: &str, _: &Path) -> Result<(), String> {
        unreachable!("two installs are rejected before any probe")
    }
    fn verify(&mut self, _: &Path, _: Option<&str>) -> Result<(), String> {
        unreachable!("two installs are rejected before any probe")
    }
    fn install(&mut self, _: &Path) -> Result<i32, String> {
        unreachable!("two installs are rejected before any probe")
    }
    fn install_msix(&mut self, _: &Path) -> Result<(), String> {
        unreachable!("two installs are rejected before any probe")
    }
    fn remove(&mut self, _: &Path) {
        unreachable!("two installs are rejected before any probe")
    }
}

#[test]
fn runs_winget_then_choco_and_strips_msstore_prefix() {
    let plan = plan(&["msstore:9NT1R1C2HH7J", "Git.Git"], &["git"]);
    let mut calls: Vec<(String, Vec<String>)> = Vec::new();

    let reports = run_plan_with(
        &plan,
        |program, args| {
            calls.push((program.to_owned(), args.iter().map(|a| (*a).to_owned()).collect()));
            Some((0, String::new()))
        },
        &mut NoMsi,
        |_| {},
    );

    let programs: Vec<&str> = calls.iter().map(|(program, _)| program.as_str()).collect();
    assert_eq!(programs, ["winget", "winget", "choco"]);
    assert_eq!(
        calls[0].1,
        [
            "install",
            "--id",
            "9NT1R1C2HH7J",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--source",
            "msstore",
            "--silent",
        ]
    );
    assert_eq!(calls[2].1, ["install", "git", "-y", "--no-progress"]);

    let packages: Vec<&str> = reports.iter().map(|r| r.package.as_str()).collect();
    assert_eq!(packages, ["9NT1R1C2HH7J", "Git.Git", "git"]);
    assert!(reports.iter().all(|r| r.outcome == Outcome::Succeeded));
}

#[test]
fn reports_are_delivered_in_order_as_they_finish() {
    let plan = plan(&["a"], &["b"]);
    let mut seen = Vec::new();

    let reports = run_plan_with(
        &plan,
        |_, _| Some((1, "boom".to_owned())),
        &mut NoMsi,
        |report| seen.push((report.manager, report.package.clone())),
    );

    assert_eq!(
        seen,
        [(Manager::Winget, "a".to_owned()), (Manager::Choco, "b".to_owned())]
    );
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|r| r.outcome == Outcome::Failed));
    assert!(reports.iter().all(|r| r.tail == ["boom"]));
}

#[test]
fn package_that_cannot_start_is_reported_failed_and_run_continues() {
    let plan = plan(&["Git.Git"], &["git"]);

    let reports = run_plan_with(
        &plan,
        |program, _| if program == "winget" { None } else { Some((2, String::new())) },
        &mut NoMsi,
        |_| {},
    );

    assert_eq!(reports[0].outcome, Outcome::Failed);
    assert_eq!(reports[0].detail, "WinGet did not start");
    assert_eq!(reports[1].outcome, Outcome::Skipped);
    assert_eq!(reports[1].detail, "nothing to do");
}

#[test]
fn powershell_without_msi_install_falls_through_to_winget() {
    let plan = plan(&["Microsoft.PowerShell"], &[]);
    let mut calls: Vec<Vec<String>> = Vec::new();

    let reports = run_plan_with(
        &plan,
        |_, args| {
            calls.push(args.iter().map(|a| (*a).to_owned()).collect());
            Some((0, String::new()))
        },
        &mut NoMsi,
        |_| {},
    );

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][1], "--id");
    assert_eq!(calls[0][2], "Microsoft.PowerShell");
    assert_eq!(reports[0].manager, Manager::Winget);
    assert_eq!(reports[0].outcome, Outcome::Succeeded);
}

#[test]
fn powershell_with_two_msi_installs_fails_without_running_winget() {
    let plan = plan(&["Microsoft.PowerShell"], &[]);
    let mut calls = 0;

    let reports = run_plan_with(
        &plan,
        |_, _| {
            calls += 1;
            Some((0, String::new()))
        },
        &mut TwoMsiInstalls,
        |_| {},
    );

    assert_eq!(calls, 0);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].manager, Manager::Msi);
    assert_eq!(reports[0].outcome, Outcome::Failed);
}

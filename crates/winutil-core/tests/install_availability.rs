//! Manager availability and skipping, checked with temporary directories and the shipped config.
//! Nothing is installed and no real PATH lookup of winget or choco is asserted.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use winutil_core::config::load;
use winutil_core::install::run::{Manager, Outcome};
use winutil_core::install::{
    Availability, Needs, PackageManager, Plan, find_program, needs, skip_unavailable,
};

/// A fresh temporary directory, removed when dropped.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("winutil-rs-avail-{}-{nanos:x}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn plan_of(winget: &[&str], choco: &[&str]) -> Plan {
    Plan {
        winget: winget.iter().map(|id| (*id).to_owned()).collect(),
        choco: choco.iter().map(|id| (*id).to_owned()).collect(),
    }
}

#[test]
fn find_program_matches_an_exe_in_a_path_directory() {
    let dir = ScratchDir::new();
    fs::write(dir.0.join("fakeprog.exe"), b"").unwrap();
    let path: OsString = std::env::join_paths([PathBuf::from("Z:/missing"), dir.0.clone()]).unwrap();

    assert!(find_program("fakeprog", &path));
}

#[test]
fn find_program_is_false_when_the_exe_is_absent() {
    let dir = ScratchDir::new();
    fs::write(dir.0.join("otherprog.exe"), b"").unwrap();
    let path: OsString = std::env::join_paths([dir.0.clone()]).unwrap();

    assert!(!find_program("fakeprog", &path));
    assert!(!find_program("fakeprog", &OsString::new()));
}

#[test]
fn find_program_ignores_a_directory_named_like_the_exe() {
    let dir = ScratchDir::new();
    fs::create_dir_all(dir.0.join("fakeprog.exe")).unwrap();
    let path: OsString = std::env::join_paths([dir.0.clone()]).unwrap();

    assert!(!find_program("fakeprog", &path));
}

#[test]
fn needs_reports_the_manager_the_app_would_use() {
    let cfg = load().unwrap();
    let app = &cfg.applications["githubdesktop"];

    assert_eq!(
        needs(app, PackageManager::Choco),
        Needs { winget: false, choco: true }
    );
    assert_eq!(
        needs(app, PackageManager::Winget),
        Needs { winget: true, choco: false }
    );
}

#[test]
fn skip_unavailable_keeps_the_plan_when_both_managers_are_present() {
    let original = plan_of(&["Git.Git"], &["git"]);
    let (kept, skipped) = skip_unavailable(
        original.clone(),
        Availability { winget: true, choco: true },
    );

    assert_eq!(kept, original);
    assert!(skipped.is_empty());
}

#[test]
fn skip_unavailable_skips_winget_packages_with_an_install_hint() {
    let (kept, skipped) = skip_unavailable(
        plan_of(&["msstore:9NT1R1C2HH7J", "Git.Git"], &["git"]),
        Availability { winget: false, choco: true },
    );

    assert!(kept.winget.is_empty());
    assert_eq!(kept.choco.into_iter().collect::<Vec<_>>(), ["git"]);
    let packages: Vec<&str> = skipped.iter().map(|r| r.package.as_str()).collect();
    assert_eq!(packages, ["9NT1R1C2HH7J", "Git.Git"]);
    for report in &skipped {
        assert_eq!(report.manager, Manager::Winget);
        assert_eq!(report.outcome, Outcome::Skipped);
        assert_eq!(report.exit_code, -1);
        assert!(report.detail.contains("WinGet is not installed"), "{}", report.detail);
        assert!(report.tail.is_empty());
    }
}

#[test]
fn skip_unavailable_skips_choco_packages_with_an_install_hint() {
    let (kept, skipped) = skip_unavailable(
        plan_of(&["Git.Git"], &["git", "github-desktop"]),
        Availability { winget: true, choco: false },
    );

    assert_eq!(kept.winget.into_iter().collect::<Vec<_>>(), ["Git.Git"]);
    assert!(kept.choco.is_empty());
    let packages: Vec<&str> = skipped.iter().map(|r| r.package.as_str()).collect();
    assert_eq!(packages, ["git", "github-desktop"]);
    assert!(skipped.iter().all(|r| r.manager == Manager::Choco));
    assert!(skipped.iter().all(|r| r.detail.contains("Chocolatey is not installed")));
}

#[test]
fn skip_unavailable_with_neither_manager_skips_everything_in_order() {
    let (kept, skipped) = skip_unavailable(
        plan_of(&["Git.Git"], &["git"]),
        Availability { winget: false, choco: false },
    );

    assert_eq!(kept, Plan::default());
    let managers: Vec<Manager> = skipped.iter().map(|r| r.manager).collect();
    assert_eq!(managers, [Manager::Winget, Manager::Choco]);
}

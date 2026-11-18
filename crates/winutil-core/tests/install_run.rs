//! Install orchestration checked with a fake process runner, so no software is installed.

use winutil_core::install::Plan;
use winutil_core::install::run::{Manager, Outcome, run_plan_with};

fn plan(winget: &[&str], choco: &[&str]) -> Plan {
    Plan {
        winget: winget.iter().map(|id| (*id).to_owned()).collect(),
        choco: choco.iter().map(|id| (*id).to_owned()).collect(),
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
        |_| {},
    );

    assert_eq!(reports[0].outcome, Outcome::Failed);
    assert_eq!(reports[0].detail, "WinGet did not start");
    assert_eq!(reports[1].outcome, Outcome::Skipped);
    assert_eq!(reports[1].detail, "nothing to do");
}

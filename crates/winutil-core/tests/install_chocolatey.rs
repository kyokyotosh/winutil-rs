//! Chocolatey bootstrap decisions, tested with fake download and process closures. Nothing here
//! downloads or runs the real installer.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use winutil_core::install::chocolatey::{INSTALL_SCRIPT_URL, install_with, powershell_command};
use winutil_core::install::run::{Manager, Outcome};

fn script() -> PathBuf {
    PathBuf::from(r"C:\Temp\winutil-rs\install-chocolatey.ps1")
}

/// `is_installed` that reports `false` on the first check and `after` on every later check.
fn installed_after(after: bool) -> impl Fn() -> bool {
    let checks = Cell::new(0);
    move || {
        let n = checks.get();
        checks.set(n + 1);
        n > 0 && after
    }
}

#[test]
fn already_installed_skips_without_download_or_run() {
    let downloads = Cell::new(0);
    let runs = Cell::new(0);

    let report = install_with(
        &script(),
        || true,
        |_, _| {
            downloads.set(downloads.get() + 1);
            Ok(())
        },
        |_, _| {
            runs.set(runs.get() + 1);
            Some((0, String::new()))
        },
    );

    assert_eq!(report.outcome, Outcome::Skipped);
    assert_eq!(downloads.get(), 0);
    assert_eq!(runs.get(), 0);
}

#[test]
fn success_downloads_official_script_and_runs_it_in_powershell() {
    let downloaded = RefCell::new(None);
    let run_args = RefCell::new(None);

    let report = install_with(
        &script(),
        installed_after(true),
        |url, dest| {
            *downloaded.borrow_mut() = Some((url.to_owned(), dest.to_path_buf()));
            Ok(())
        },
        |program: &str, args: &[&str]| {
            *run_args.borrow_mut() = Some((
                program.to_owned(),
                args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>(),
            ));
            Some((0, String::new()))
        },
    );

    assert_eq!(report.outcome, Outcome::Succeeded);
    assert_eq!(report.detail, "Chocolatey installed.");
    assert_eq!(report.manager, Manager::Choco);
    assert_eq!(report.exit_code, 0);
    assert!(report.tail.is_empty());

    let (url, dest) = downloaded.borrow().clone().expect("script was downloaded");
    assert_eq!(url, "https://community.chocolatey.org/install.ps1");
    assert_eq!(url, INSTALL_SCRIPT_URL);
    assert_eq!(dest, script());

    let (program, args) = run_args.borrow().clone().expect("installer was run");
    assert_eq!(program, "powershell.exe");
    assert_eq!(
        args,
        [
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            powershell_command(&script()).as_str(),
        ]
    );
}

#[test]
fn download_failure_reports_url_and_does_not_run() {
    let runs = Cell::new(0);

    let report = install_with(
        &script(),
        || false,
        |_, _| Err("connection reset".to_owned()),
        |_, _| {
            runs.set(runs.get() + 1);
            Some((0, String::new()))
        },
    );

    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(
        report.detail,
        "could not download https://community.chocolatey.org/install.ps1: connection reset"
    );
    assert_eq!(runs.get(), 0);
}

#[test]
fn powershell_that_does_not_start_is_a_failure() {
    let report = install_with(&script(), || false, |_, _| Ok(()), |_, _| None);

    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.exit_code, -1);
    assert_eq!(
        report.detail,
        "could not start powershell.exe to run the Chocolatey installer"
    );
}

#[test]
fn nonzero_installer_exit_keeps_output_tail() {
    let report = install_with(
        &script(),
        || false,
        |_, _| Ok(()),
        |_, _| Some((1, "setup line\nAdministrator rights are required.\n".to_owned())),
    );

    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.exit_code, 1);
    assert_eq!(report.detail, "the Chocolatey installer exited with code 1.");
    assert_eq!(
        report.tail,
        ["setup line", "Administrator rights are required."]
    );
}

#[test]
fn clean_exit_without_choco_on_path_is_a_failure() {
    let report = install_with(
        &script(),
        installed_after(false),
        |_, _| Ok(()),
        |_, _| Some((0, "done".to_owned())),
    );

    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(
        report.detail,
        "Chocolatey was installed but choco is still not on PATH."
    );
}

#[test]
fn powershell_command_enables_tls12_and_quotes_path() {
    let command = powershell_command(Path::new(r"C:\Users\O'Brien\install.ps1"));

    assert!(command.contains("SecurityProtocol -bor 3072"), "{command}");
    assert!(
        command.ends_with(r"; & 'C:\Users\O''Brien\install.ps1'"),
        "{command}"
    );
}

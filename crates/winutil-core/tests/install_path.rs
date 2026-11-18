//! PATH refresh helpers for tools installed while the process is running.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use winutil_core::install::path::{expand_env, merge_path};
use winutil_core::install::run::run_command_in;

fn entries(path: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(path).collect()
}

#[test]
fn merge_appends_only_new_directories() {
    let merged = merge_path(
        OsStr::new(r"C:\Windows;C:\Tools"),
        &[r"C:\Tools\;C:\ProgramData\chocolatey\bin"],
    );

    assert_eq!(
        entries(&merged),
        [
            PathBuf::from(r"C:\Windows"),
            PathBuf::from(r"C:\Tools"),
            PathBuf::from(r"C:\ProgramData\chocolatey\bin"),
        ]
    );
}

#[test]
fn merge_ignores_case_and_trailing_separator_when_comparing() {
    let merged = merge_path(OsStr::new(r"c:\windows"), &[r"C:\Windows\", r"C:\WINDOWS;;"]);

    assert_eq!(entries(&merged), [PathBuf::from(r"c:\windows")]);
}

#[test]
fn merge_keeps_current_path_when_persisted_is_empty() {
    let current = OsString::from(r"C:\Windows;C:\Tools");
    let merged = merge_path(&current, &["", ""]);

    assert_eq!(merged, current);
}

#[test]
fn expand_env_replaces_known_references() {
    let expanded = expand_env(r"%ProgramData%\chocolatey\bin;%MISSING%\x", |name| {
        (name == "ProgramData").then(|| r"C:\ProgramData".to_owned())
    });

    assert_eq!(expanded, r"C:\ProgramData\chocolatey\bin;%MISSING%\x");
}

#[test]
fn expand_env_keeps_unmatched_percent_signs() {
    assert_eq!(expand_env("100%", |_| Some("x".into())), "100%");
    assert_eq!(expand_env("a%%b", |_| None), "a%%b");
}

/// A directory under the system temp dir that is removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "winutil-path-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn spawn_resolves_program_through_the_given_path() {
    let dir = TempDir::new("resolve");
    // `winutil_probe` exists only in `dir`, so finding it proves the given PATH was searched.
    let system_cmd = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_default())
        .join("System32")
        .join("cmd.exe");
    fs::copy(&system_cmd, dir.path().join("winutil_probe.exe")).expect("copy cmd.exe");

    let result = run_command_in("winutil_probe", &["/C", "exit", "7"], dir.path().as_os_str());

    assert_eq!(result.map(|(code, _)| code), Some(7));
}

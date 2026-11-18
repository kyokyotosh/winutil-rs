//! The `PATH` used to find install tools. This process starts with the `PATH` it inherited, so a
//! tool an installer adds later (such as `choco.exe`) is not found until the process restarts.
//! The machine and user `PATH` in the registry hold the installer's change, so they are merged in.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

const MACHINE_ENVIRONMENT: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const USER_ENVIRONMENT: &str = "Environment";

/// The `PATH` a new process should search: this process's `PATH`, then the machine and user `PATH`
/// from the registry, skipping entries already listed.
pub fn search_path() -> OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let machine = read_persisted(RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(MACHINE_ENVIRONMENT));
    let user = read_persisted(RegKey::predef(HKEY_CURRENT_USER).open_subkey(USER_ENVIRONMENT));
    merge_path(&current, &[machine.as_str(), user.as_str()])
}

/// `current` followed by each directory in `persisted` that it does not already list. Directories
/// compare case-insensitively, ignoring a trailing backslash, the way Windows compares paths.
pub fn merge_path(current: &OsStr, persisted: &[&str]) -> OsString {
    let mut entries: Vec<PathBuf> = std::env::split_paths(current)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect();
    let mut seen: HashSet<String> = entries.iter().map(|dir| comparison_key(dir)).collect();
    for raw in persisted {
        for dir in std::env::split_paths(raw) {
            if !dir.as_os_str().is_empty() && seen.insert(comparison_key(&dir)) {
                entries.push(dir);
            }
        }
    }
    // join_paths refuses a directory containing a double quote; keep the current PATH rather than
    // drop it entirely.
    std::env::join_paths(&entries).unwrap_or_else(|_| current.to_owned())
}

/// Replaces each `%NAME%` in `value` with `lookup(NAME)`. A reference that `lookup` cannot resolve
/// is kept as written, and so is an unmatched `%`.
pub fn expand_env(value: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else { break };
        let name = &after[..end];
        out.push_str(&rest[..start]);
        match lookup(name) {
            Some(text) if !name.is_empty() => out.push_str(&text),
            _ => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn read_persisted(key: io::Result<RegKey>) -> String {
    key.and_then(|key| key.get_value::<String, _>("Path"))
        .map(|raw| expand_env(&raw, |name| std::env::var(name).ok()))
        .unwrap_or_default()
}

fn comparison_key(dir: &Path) -> String {
    dir.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

//! Install planning: turns selected applications into winget and Chocolatey package IDs.
//!
//! Mirrors `Get-WinUtilSelectedPackages` and the install command lines in the PowerShell
//! original. Running the commands lives in [`run`].

pub mod run;

use indexmap::IndexSet;

use crate::config::applications::Application;

/// The package manager the user prefers. Applications without a usable Chocolatey ID fall back
/// to WinGet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Winget,
    Choco,
}

/// Package IDs to install, split by manager. Each set is deduplicated and keeps selection order.
/// WinGet IDs keep an `msstore:` prefix when they come from the Microsoft Store.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    pub winget: IndexSet<String>,
    pub choco: IndexSet<String>,
}

/// Builds the install plan for `apps` under `preference`.
pub fn plan<'a>(apps: impl IntoIterator<Item = &'a Application>, preference: PackageManager) -> Plan {
    let mut plan = Plan::default();
    for app in apps {
        match preference {
            PackageManager::Winget => push_ids(&mut plan.winget, &app.winget),
            PackageManager::Choco => match app.choco.as_deref() {
                Some(choco) if is_usable(choco) => push_ids(&mut plan.choco, choco),
                _ => push_ids(&mut plan.winget, &app.winget),
            },
        }
    }
    plan
}

/// Splits an `msstore:` prefix off a WinGet ID, returning the source and the bare ID.
pub fn winget_source(raw: &str) -> (&'static str, &str) {
    match raw.get(..8) {
        Some(prefix) if prefix.eq_ignore_ascii_case("msstore:") => ("msstore", &raw[8..]),
        _ => ("winget", raw),
    }
}

/// Arguments for `winget` that install one package from `source`.
pub fn winget_install_args<'a>(source: &'a str, id: &'a str) -> [&'a str; 8] {
    [
        "install",
        "--id",
        id,
        "--accept-package-agreements",
        "--accept-source-agreements",
        "--source",
        source,
        "--silent",
    ]
}

/// Arguments for `choco` that install one package without prompts.
pub fn choco_install_args(id: &str) -> [&str; 4] {
    ["install", id, "-y", "--no-progress"]
}

/// A Chocolatey field is usable when it is not blank and not the `na` placeholder.
fn is_usable(choco: &str) -> bool {
    !choco.trim().is_empty() && !choco.eq_ignore_ascii_case("na")
}

/// Adds every ID in a `;`-separated field, skipping blanks, `na`, and duplicates.
fn push_ids(target: &mut IndexSet<String>, raw: &str) {
    for id in raw.split(';').map(str::trim) {
        if !id.is_empty() && !id.eq_ignore_ascii_case("na") {
            target.insert(id.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(winget: &str, choco: Option<&str>) -> Application {
        Application {
            comment: None,
            category: String::new(),
            content: String::new(),
            description: String::new(),
            link: String::new(),
            winget: winget.to_owned(),
            choco: choco.map(str::to_owned),
            foss: None,
        }
    }

    fn set(ids: &[&str]) -> IndexSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn splits_semicolon_ids_and_keeps_order() {
        let apps = [app("a; b", None)];
        let plan = plan(&apps, PackageManager::Winget);
        assert_eq!(plan.winget, set(&["a", "b"]));
    }

    #[test]
    fn skips_blank_and_na_ids_case_insensitively() {
        let apps = [app(" ;NA; x ;", None)];
        let plan = plan(&apps, PackageManager::Winget);
        assert_eq!(plan.winget, set(&["x"]));
    }

    #[test]
    fn deduplicates_across_applications() {
        let apps = [app("x", None), app("x; y", None)];
        let plan = plan(&apps, PackageManager::Winget);
        assert_eq!(plan.winget, set(&["x", "y"]));
    }

    #[test]
    fn winget_preference_ignores_choco() {
        let apps = [app("w", Some("c"))];
        let plan = plan(&apps, PackageManager::Winget);
        assert_eq!(plan.winget, set(&["w"]));
        assert!(plan.choco.is_empty());
    }

    #[test]
    fn choco_preference_uses_choco_id_when_present() {
        let apps = [app("w", Some("git;github-desktop"))];
        let plan = plan(&apps, PackageManager::Choco);
        assert_eq!(plan.choco, set(&["git", "github-desktop"]));
        assert!(plan.winget.is_empty());
    }

    #[test]
    fn choco_preference_falls_back_to_winget_for_missing_or_na_choco() {
        let apps = [app("w1", None), app("w2", Some(" ")), app("w3", Some("NA"))];
        let plan = plan(&apps, PackageManager::Choco);
        assert_eq!(plan.winget, set(&["w1", "w2", "w3"]));
        assert!(plan.choco.is_empty());
    }

    #[test]
    fn msstore_prefix_selects_store_source_and_strips_prefix() {
        assert_eq!(winget_source("msstore:9NT1R1C2HH7J"), ("msstore", "9NT1R1C2HH7J"));
        assert_eq!(winget_source("MSSTORE:abc"), ("msstore", "abc"));
        assert_eq!(winget_source("Git.Git"), ("winget", "Git.Git"));
        assert_eq!(winget_source("msstor"), ("winget", "msstor"));
    }

    #[test]
    fn winget_install_args_match_original_command() {
        assert_eq!(
            winget_install_args("winget", "Git.Git"),
            [
                "install",
                "--id",
                "Git.Git",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--source",
                "winget",
                "--silent",
            ]
        );
    }

    #[test]
    fn choco_install_args_match_original_command() {
        assert_eq!(choco_install_args("git"), ["install", "git", "-y", "--no-progress"]);
    }
}

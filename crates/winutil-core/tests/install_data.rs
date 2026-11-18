//! Install planning checked against the shipped applications.

use winutil_core::config::load;
use winutil_core::install::{PackageManager, plan};

const PREFERENCES: [PackageManager; 2] = [PackageManager::Winget, PackageManager::Choco];

#[test]
fn every_application_resolves_to_an_id_under_both_preferences() {
    let cfg = load().unwrap();
    for (key, app) in &cfg.applications {
        for preference in PREFERENCES {
            let plan = plan([app], preference);
            assert!(
                !plan.winget.is_empty() || !plan.choco.is_empty(),
                "{key} has no package id under {preference:?}"
            );
        }
    }
}

#[test]
fn planned_ids_are_trimmed_and_free_of_separators_and_placeholders() {
    let cfg = load().unwrap();
    for preference in PREFERENCES {
        let plan = plan(cfg.applications.values(), preference);
        for id in plan.winget.iter().chain(&plan.choco) {
            assert!(
                !id.is_empty() && id == id.trim() && !id.contains(';') && !id.eq_ignore_ascii_case("na"),
                "bad id {id:?} under {preference:?}"
            );
        }
    }
}

#[test]
fn choco_preference_splits_dependency_ids_from_shipped_config() {
    let cfg = load().unwrap();
    let app = &cfg.applications["githubdesktop"];
    let plan = plan([app], PackageManager::Choco);
    assert_eq!(plan.choco.into_iter().collect::<Vec<_>>(), ["git", "github-desktop"]);
}

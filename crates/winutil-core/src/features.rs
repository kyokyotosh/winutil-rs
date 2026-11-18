//! Features tab: Windows optional features enabled through DISM, then each entry's own scripts.
//!
//! The decisions live here and are tested with a fake [`FeatureHost`]. The production host is in
//! [`system`].

pub mod system;

use indexmap::IndexMap;

use crate::config::feature::Feature;

/// The category the Features tab shows. Other categories belong to other tabs.
const CATEGORY: &str = "Features";

/// Result of installing one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    pub detail: String,
}

/// The side effects an entry needs. The production host runs DISM and PowerShell.
pub trait FeatureHost {
    /// Enables one DISM optional feature, with everything it depends on.
    fn enable(&mut self, name: &str) -> Result<(), String>;
    /// Runs one PowerShell script from the entry's `InvokeScript`.
    fn run_script(&mut self, script: &str) -> Result<(), String>;
}

/// The Features tab's checkboxes, in file order. The "Install Features" button is excluded.
pub fn checkboxes(features: &IndexMap<String, Feature>) -> Vec<(&str, &Feature)> {
    features
        .iter()
        .filter(|(_, feature)| feature.category == CATEGORY && feature.kind.is_none())
        .map(|(key, feature)| (key.as_str(), feature))
        .collect()
}

/// Installs one entry. Every feature is attempted even after one fails, as the original does.
/// The scripts run only when all features were enabled. A failing script stops the rest.
pub fn install(feature: &Feature, host: &mut impl FeatureHost) -> Report {
    let mut failures = Vec::new();
    for name in &feature.optional_features {
        if let Err(error) = host.enable(name) {
            failures.push(format!("{name}: {error}"));
        }
    }
    if !failures.is_empty() {
        return Report {
            outcome: Outcome::Failed,
            detail: format!("{}; scripts skipped", failures.join("; ")),
        };
    }
    for script in &feature.invoke_script {
        if let Err(error) = host.run_script(script) {
            return Report {
                outcome: Outcome::Failed,
                detail: format!("script failed: {error}"),
            };
        }
    }
    Report {
        outcome: Outcome::Done,
        detail: summary(feature),
    }
}

fn summary(feature: &Feature) -> String {
    let mut parts = Vec::new();
    if !feature.optional_features.is_empty() {
        parts.push(format!("enabled {}", feature.optional_features.join(", ")));
    }
    if !feature.invoke_script.is_empty() {
        parts.push(format!("ran {} script(s)", feature.invoke_script.len()));
    }
    if parts.is_empty() {
        return "nothing to do".to_owned();
    }
    parts.join("; ")
}

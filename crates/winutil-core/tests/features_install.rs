use winutil_core::config::{self, feature::Feature};
use winutil_core::features::{self, FeatureHost, Outcome};

/// Records every call and fails the names and scripts it is told to.
#[derive(Default)]
struct Fake {
    fail_enable: Vec<&'static str>,
    fail_script: Option<&'static str>,
    calls: Vec<String>,
}

impl FeatureHost for Fake {
    fn enable(&mut self, name: &str) -> Result<(), String> {
        self.calls.push(format!("enable {name}"));
        if self.fail_enable.contains(&name) {
            return Err("exit code 1".to_owned());
        }
        Ok(())
    }

    fn run_script(&mut self, script: &str) -> Result<(), String> {
        self.calls.push(format!("script {script}"));
        if self.fail_script == Some(script) {
            return Err("exit code 1: boom".to_owned());
        }
        Ok(())
    }
}

fn entry(features: &[&str], scripts: &[&str]) -> Feature {
    Feature {
        category: "Features".to_owned(),
        content: "test".to_owned(),
        description: None,
        link: String::new(),
        panel: 1,
        kind: None,
        button_width: None,
        optional_features: features.iter().map(|s| (*s).to_owned()).collect(),
        invoke_script: scripts.iter().map(|s| (*s).to_owned()).collect(),
        function: None,
    }
}

#[test]
fn enables_each_feature_then_runs_scripts_in_order() {
    let mut host = Fake::default();
    let report = features::install(&entry(&["A", "B"], &["s1", "s2"]), &mut host);
    assert_eq!(report.outcome, Outcome::Done);
    assert_eq!(report.detail, "enabled A, B; ran 2 script(s)");
    assert_eq!(
        host.calls,
        ["enable A", "enable B", "script s1", "script s2"]
    );
}

#[test]
fn failed_enable_skips_scripts_but_still_tries_the_other_features() {
    let mut host = Fake {
        fail_enable: vec!["A"],
        ..Fake::default()
    };
    let report = features::install(&entry(&["A", "B"], &["s1"]), &mut host);
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.detail, "A: exit code 1; scripts skipped");
    assert_eq!(host.calls, ["enable A", "enable B"]);
}

#[test]
fn failing_script_stops_the_remaining_scripts() {
    let mut host = Fake {
        fail_script: Some("s1"),
        ..Fake::default()
    };
    let report = features::install(&entry(&[], &["s1", "s2"]), &mut host);
    assert_eq!(report.outcome, Outcome::Failed);
    assert_eq!(report.detail, "script failed: exit code 1: boom");
    assert_eq!(host.calls, ["script s1"]);
}

#[test]
fn script_only_entry_runs_its_script() {
    let mut host = Fake::default();
    let report = features::install(
        &entry(&[], &["bcdedit /set bootmenupolicy legacy"]),
        &mut host,
    );
    assert_eq!(report.outcome, Outcome::Done);
    assert_eq!(report.detail, "ran 1 script(s)");
    assert_eq!(host.calls, ["script bcdedit /set bootmenupolicy legacy"]);
}

#[test]
fn checkboxes_are_the_features_entries_without_the_install_button() {
    let config = config::load().expect("embedded config is valid");
    let keys: Vec<&str> = features::checkboxes(&config.features)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(keys.len(), 9);
    assert!(!keys.contains(&"WPFFeatureInstall"));
    assert!(keys.contains(&"WPFFeaturesdotnet"));
    assert!(keys.contains(&"WPFFeatureDisableLegacyRecovery"));
}

#[test]
fn every_checkbox_has_something_to_do() {
    let config = config::load().expect("embedded config is valid");
    for (key, feature) in features::checkboxes(&config.features) {
        assert!(
            !feature.optional_features.is_empty() || !feature.invoke_script.is_empty(),
            "{key} has neither a feature nor a script"
        );
    }
}

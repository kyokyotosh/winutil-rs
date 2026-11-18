//! Checks on the real `config/*.json` data. A failure here means the shipped data is wrong or the
//! model no longer matches it.

use std::collections::BTreeMap;

use winutil_core::config::navigation::NavKind;
use winutil_core::config::registry::{RegType, RegValue};
use winutil_core::config::tweaks::WidgetKind;
use winutil_core::config::{Config, load};

fn config() -> Config {
    load().expect("embedded config must parse")
}

#[test]
fn known_tweak_parses_registry_fields() {
    let cfg = config();
    let change = &cfg.tweaks["WPFTweaksActivity"].registry[0];

    assert_eq!(change.path, r"HKLM:\SOFTWARE\Policies\Microsoft\Windows\System");
    assert_eq!(change.name, "EnableActivityFeed");
    assert_eq!(change.kind, RegType::DWord);
    assert_eq!(change.value, Some(RegValue::Set("1".into())));
    assert_eq!(change.original_value, Some(RegValue::Remove));
}

#[test]
fn every_preset_entry_resolves_to_a_tweak_or_appx_package() {
    let cfg = config();
    for (preset, keys) in &cfg.presets {
        for key in keys {
            assert!(
                cfg.tweaks.contains_key(key) || cfg.appx.contains_key(key),
                "preset {preset} references unknown entry {key}"
            );
        }
    }
}

#[test]
fn combobox_descriptions_and_registry_values_match_choices() {
    let cfg = config();
    let comboboxes = cfg
        .tweaks
        .iter()
        .filter(|(_, tweak)| tweak.kind == Some(WidgetKind::Combobox));

    for (key, tweak) in comboboxes {
        let choices = tweak.combo_choices();
        assert!(!choices.is_empty(), "{key}: combobox has no ComboItems");

        for description in tweak.combo_descriptions.keys() {
            assert!(
                choices.contains(&description.as_str()),
                "{key}: description for unknown choice {description}"
            );
        }
        for change in &tweak.registry {
            for choice in change.values.keys() {
                assert!(
                    choices.contains(&choice.as_str()),
                    "{key}: registry value for unknown choice {choice}"
                );
            }
        }
    }
}

#[test]
fn every_application_has_a_package_id_and_https_link() {
    let cfg = config();
    for (name, app) in &cfg.applications {
        let has_id = !app.winget.is_empty() || app.choco.as_deref().is_some_and(|id| !id.is_empty());
        assert!(has_id, "{name}: no winget or choco id");
        assert!(app.link.starts_with("https://"), "{name}: link is not https");
    }
}

#[test]
fn every_appx_package_has_a_package_id() {
    let cfg = config();
    for (key, package) in &cfg.appx {
        assert!(!package.package_id.is_empty(), "{key}: empty PackageId");
    }
}

#[test]
fn every_feature_has_something_to_run() {
    let cfg = config();
    for (key, feature) in &cfg.features {
        let runs_something =
            feature.function.is_some() || !feature.optional_features.is_empty() || !feature.invoke_script.is_empty();
        assert!(runs_something, "{key}: nothing to run");
    }
}

#[test]
fn every_dns_provider_has_primary_and_secondary_servers() {
    let cfg = config();
    for (key, provider) in &cfg.dns {
        assert!(!provider.primary.is_empty(), "{key}: empty primary");
        assert!(!provider.secondary.is_empty(), "{key}: empty secondary");
    }
}

#[test]
fn at_most_one_checked_radio_button_per_group() {
    let cfg = config();
    let mut checked_per_group: BTreeMap<&str, usize> = BTreeMap::new();
    for item in cfg.app_navigation.values() {
        if item.kind == NavKind::RadioButton && item.checked {
            if let Some(group) = &item.group_name {
                *checked_per_group.entry(group).or_default() += 1;
            }
        }
    }
    for (group, count) in checked_per_group {
        assert!(count <= 1, "group {group} has {count} checked radio buttons");
    }
}

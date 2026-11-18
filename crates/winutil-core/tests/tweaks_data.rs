//! Checks the vendored `tweaks.json` against the tweak runner's assumptions. A failure here means
//! an entry the runner would silently skip, or a button it has no action for.

use winutil_core::config::{self, tweaks::WidgetKind};
use winutil_core::tweaks::{Direction, combo_writes, plan, split_path};

/// Buttons the runner has an action for. Keys match the `tweaks.json` names.
const BUTTONS: [&str; 3] = ["WPFOOSUbutton", "WPFAddUltPerf", "WPFRemoveUltPerf"];

fn config() -> config::Config {
    config::load().expect("embedded config parses")
}

#[test]
fn every_registry_path_names_a_known_hive() {
    for (name, tweak) in &config().tweaks {
        for change in &tweak.registry {
            assert!(
                split_path(&change.path).is_some(),
                "{name}: unknown hive in {}",
                change.path
            );
        }
    }
}

#[test]
fn toggle_entries_carry_both_values_so_neither_direction_drops_them() {
    let mut toggles = 0;
    for (name, tweak) in &config().tweaks {
        if tweak.kind != Some(WidgetKind::Toggle) {
            continue;
        }
        toggles += 1;
        for change in &tweak.registry {
            assert!(change.values.is_empty(), "{name}: toggle has Values");
            assert!(change.value.is_some(), "{name}: {} has no Value", change.name);
            assert!(
                change.original_value.is_some(),
                "{name}: {} has no OriginalValue",
                change.name
            );
        }
        assert_eq!(plan(tweak, Direction::Apply).registry.len(), tweak.registry.len());
        assert_eq!(plan(tweak, Direction::Undo).registry.len(), tweak.registry.len());
    }
    assert!(toggles > 0, "the config has toggle tweaks");
}

#[test]
fn every_button_tweak_has_an_action() {
    for (name, tweak) in &config().tweaks {
        if tweak.kind == Some(WidgetKind::Button) {
            assert!(BUTTONS.contains(&name.as_str()), "{name} has no action");
        }
    }
}

#[test]
fn every_combobox_choice_has_a_write_for_each_entry() {
    for (name, tweak) in &config().tweaks {
        if tweak.kind != Some(WidgetKind::Combobox) || tweak.registry.is_empty() {
            continue;
        }
        if !tweak.registry.iter().all(|change| !change.values.is_empty()) {
            continue;
        }
        for choice in tweak.combo_choices() {
            let writes = combo_writes(&tweak.registry, choice);
            assert!(writes.is_some(), "{name}: no write for choice {choice}");
            assert_eq!(writes.unwrap().len(), tweak.registry.len());
        }
    }
}

#[test]
fn multiplane_overlay_offers_the_three_documented_choices() {
    let config = config();
    let tweak = config
        .tweaks
        .get("WPFMultiplaneOverlay")
        .expect("multiplane overlay tweak exists");
    assert_eq!(
        tweak.combo_choices(),
        ["Enabled", "Disabled (Compatibility)", "Fully Disabled"]
    );
    assert_eq!(tweak.registry.len(), 2);
}

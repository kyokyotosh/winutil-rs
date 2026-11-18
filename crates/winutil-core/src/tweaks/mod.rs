//! Windows tweaks: the pure decisions behind the Tweaks tab.
//!
//! Mirrors `Invoke-WinUtilTweaks`, `Get-WinUtilToggleStatus`, and the combobox helpers in the
//! PowerShell original. Machine access goes through [`run::TweakHost`]; nothing in this module
//! touches the registry, services, or processes.

pub mod power;
pub mod run;
pub mod system;

use crate::config::registry::{RegType, RegValue, RegistryChange};
use crate::config::tweaks::{StartupType, Tweak};

/// Whether a service whose startup type differs from what the tweak expects is left alone. The
/// original defaults to true.
pub const KEEP_SERVICE_STARTUP: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Apply,
    Undo,
}

/// A registry value write. [`RegValue::Remove`] deletes the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegWrite {
    pub path: String,
    pub name: String,
    pub kind: RegType,
    pub value: RegValue,
}

/// A service startup change. `expected` is the startup type the tweak assumes is current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceWrite {
    pub name: String,
    pub target: StartupType,
    pub expected: StartupType,
}

/// Everything one tweak does in one direction, in the order it runs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    pub registry: Vec<RegWrite>,
    pub services: Vec<ServiceWrite>,
    pub scripts: Vec<String>,
}

/// Builds the plan for `tweak`. Combobox entries (those with `Values`) are left out; they are
/// written by [`combo_writes`]. An entry with no value for this direction is left out too.
pub fn plan(tweak: &Tweak, direction: Direction) -> Plan {
    let registry = tweak
        .registry
        .iter()
        .filter(|change| change.values.is_empty())
        .filter_map(|change| {
            let value = match direction {
                Direction::Apply => change.value.clone(),
                Direction::Undo => change.original_value.clone(),
            }?;
            Some(RegWrite {
                path: change.path.clone(),
                name: change.name.clone(),
                kind: change.kind,
                value,
            })
        })
        .collect();
    let services = tweak
        .service
        .iter()
        .map(|change| {
            let (target, expected) = match direction {
                Direction::Apply => (change.startup_type, change.original_type),
                Direction::Undo => (change.original_type, change.startup_type),
            };
            ServiceWrite {
                name: change.name.clone(),
                target,
                expected,
            }
        })
        .collect();
    let scripts = match direction {
        Direction::Apply => tweak.invoke_script.clone(),
        Direction::Undo => tweak.undo_script.clone(),
    };
    Plan {
        registry,
        services,
        scripts,
    }
}

/// What happens to one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Set(StartupType),
    Unchanged,
    Missing,
    /// The service is not in the state the tweak expects, and startup is being kept.
    Mismatch(StartupType),
}

/// Decides what to do with one service given its current startup type (`None` if it does not
/// exist). A mismatch is checked before the target, as the original does.
pub fn service_action(
    write: &ServiceWrite,
    current: Option<StartupType>,
    keep_startup: bool,
) -> ServiceAction {
    let Some(current) = current else {
        return ServiceAction::Missing;
    };
    if keep_startup && current != write.expected {
        return ServiceAction::Mismatch(current);
    }
    if current == write.target {
        ServiceAction::Unchanged
    } else {
        ServiceAction::Set(write.target)
    }
}

/// A registry hive named by a `HKLM:`, `HKCU:`, or `HKU:` path prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    LocalMachine,
    CurrentUser,
    Users,
}

/// Splits `HKLM:\Sub\Key` into its hive and the subkey path. `None` for an unknown hive.
pub fn split_path(path: &str) -> Option<(Hive, &str)> {
    let (hive, rest) = path.split_once(":\\")?;
    let hive = match hive {
        "HKLM" => Hive::LocalMachine,
        "HKCU" => Hive::CurrentUser,
        "HKU" => Hive::Users,
        _ => return None,
    };
    Some((hive, rest))
}

/// Whether two stored values of type `kind` are the same. Strings compare case-insensitively and
/// numbers numerically, as PowerShell's `-ne` does.
pub fn same_text(kind: RegType, a: &str, b: &str) -> bool {
    match kind {
        RegType::String => a.eq_ignore_ascii_case(b),
        RegType::DWord | RegType::QWord => match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == b,
        },
    }
}

fn same_option(kind: RegType, a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_text(kind, a, b),
        _ => false,
    }
}

/// The text a configured value is compared as. `None` means "absent".
fn text_of(value: &RegValue) -> Option<&str> {
    match value {
        RegValue::Set(text) => Some(text),
        RegValue::Remove => None,
    }
}

/// Whether one registry entry is in its applied state, given the stored text (`None` when the
/// value is absent). Mirrors `Get-WinUtilToggleStatus`: an absent value falls back to
/// `DefaultState`. `true` counts as applied; `false` compares `OriginalValue` with `Value`.
pub fn registry_matches(change: &RegistryChange, current: Option<&str>) -> bool {
    let Some(value) = change.value.as_ref() else {
        return false;
    };
    match current {
        Some(text) => match value {
            RegValue::Set(expected) => same_text(change.kind, text, expected),
            RegValue::Remove => false,
        },
        None => match change.default_state {
            Some(true) => true,
            Some(false) => change.original_value.as_ref().is_some_and(|original| {
                same_option(change.kind, text_of(original), text_of(value))
            }),
            None => false,
        },
    }
}

/// The combobox choice whose values all match what is stored, or `None`.
///
/// `current` holds the stored text of each registry entry, in the same order as `registry`
/// (`None` when absent). An absent value is read as its `DefaultValue`. A `<RemoveEntry>` choice
/// expects the `DefaultValue`. Mirrors `Get-WinUtilRegistryComboState`.
pub fn combo_state<'a>(
    registry: &[RegistryChange],
    current: &[Option<String>],
    choices: &[&'a str],
) -> Option<&'a str> {
    choices.iter().copied().find(|choice| {
        registry.iter().zip(current).all(|(change, stored)| {
            let Some(expected) = change.values.get(*choice) else {
                return false;
            };
            let expected = match expected {
                RegValue::Set(text) => Some(text.as_str()),
                RegValue::Remove => change.default_value.as_deref(),
            };
            let actual = stored.as_deref().or(change.default_value.as_deref());
            same_option(change.kind, actual, expected)
        })
    })
}

/// The writes that select `state`: one per combobox entry. `None` if any entry has no value for
/// `state`.
pub fn combo_writes(registry: &[RegistryChange], state: &str) -> Option<Vec<RegWrite>> {
    registry
        .iter()
        .filter(|change| !change.values.is_empty())
        .map(|change| {
            let value = change.values.get(state)?.clone();
            Some(RegWrite {
                path: change.path.clone(),
                name: change.name.clone(),
                kind: change.kind,
                value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tweaks::{ServiceChange, WidgetKind};
    use indexmap::IndexMap;

    fn change(json: &str) -> RegistryChange {
        serde_json_lenient::from_str(json).unwrap()
    }

    fn tweak(registry: Vec<RegistryChange>, service: Vec<ServiceChange>) -> Tweak {
        Tweak {
            category: "Test".into(),
            content: "Test".into(),
            description: None,
            link: String::new(),
            panel: 2,
            kind: Some(WidgetKind::Toggle),
            button_width: None,
            checked: false,
            registry,
            service,
            invoke_script: vec!["apply-script".into()],
            undo_script: vec!["undo-script".into()],
            combo_items: None,
            combo_descriptions: IndexMap::new(),
        }
    }

    const TOGGLE: &str = r#"{"Path":"HKCU:\\Software\\X","Name":"Flag","Type":"DWord",
        "Value":"1","OriginalValue":"0","DefaultState":"false"}"#;

    #[test]
    fn apply_writes_value_and_undo_writes_original() {
        let t = tweak(vec![change(TOGGLE)], vec![]);
        let apply = plan(&t, Direction::Apply);
        assert_eq!(apply.registry[0].value, RegValue::Set("1".into()));
        assert_eq!(apply.scripts, ["apply-script"]);
        let undo = plan(&t, Direction::Undo);
        assert_eq!(undo.registry[0].value, RegValue::Set("0".into()));
        assert_eq!(undo.scripts, ["undo-script"]);
    }

    #[test]
    fn remove_entry_stays_a_remove_write() {
        let t = tweak(
            vec![change(
                r#"{"Path":"HKLM:\\X","Name":"A","Type":"DWord","Value":"<RemoveEntry>","OriginalValue":"1"}"#,
            )],
            vec![],
        );
        assert_eq!(plan(&t, Direction::Apply).registry[0].value, RegValue::Remove);
    }

    #[test]
    fn combobox_entries_are_not_planned_as_plain_writes() {
        let t = tweak(
            vec![change(
                r#"{"Path":"HKLM:\\X","Name":"A","Type":"DWord","DefaultValue":"0",
                    "Values":{"Enabled":"<RemoveEntry>","Off":"5"}}"#,
            )],
            vec![],
        );
        assert!(plan(&t, Direction::Apply).registry.is_empty());
    }

    #[test]
    fn service_is_planned_with_its_expected_state_in_each_direction() {
        let service = ServiceChange {
            name: "svc".into(),
            startup_type: StartupType::Disabled,
            original_type: StartupType::Manual,
        };
        let t = tweak(vec![], vec![service]);
        let apply = plan(&t, Direction::Apply);
        assert_eq!(
            apply.services[0],
            ServiceWrite {
                name: "svc".into(),
                target: StartupType::Disabled,
                expected: StartupType::Manual
            }
        );
        let undo = plan(&t, Direction::Undo);
        assert_eq!(
            undo.services[0],
            ServiceWrite {
                name: "svc".into(),
                target: StartupType::Manual,
                expected: StartupType::Disabled
            }
        );
    }

    fn svc(target: StartupType, expected: StartupType) -> ServiceWrite {
        ServiceWrite {
            name: "svc".into(),
            target,
            expected,
        }
    }

    #[test]
    fn service_missing_is_reported_before_anything_else() {
        let w = svc(StartupType::Disabled, StartupType::Manual);
        assert_eq!(service_action(&w, None, true), ServiceAction::Missing);
        assert_eq!(service_action(&w, None, false), ServiceAction::Missing);
    }

    #[test]
    fn service_in_unexpected_state_is_kept_only_when_keep_startup_is_on() {
        let w = svc(StartupType::Disabled, StartupType::Manual);
        assert_eq!(
            service_action(&w, Some(StartupType::Automatic), true),
            ServiceAction::Mismatch(StartupType::Automatic)
        );
        assert_eq!(
            service_action(&w, Some(StartupType::Automatic), false),
            ServiceAction::Set(StartupType::Disabled)
        );
    }

    #[test]
    fn service_already_at_target_is_unchanged() {
        let w = svc(StartupType::Disabled, StartupType::Manual);
        assert_eq!(
            service_action(&w, Some(StartupType::Disabled), false),
            ServiceAction::Unchanged
        );
    }

    #[test]
    fn present_value_must_equal_value_to_be_applied() {
        let c = change(TOGGLE);
        assert!(registry_matches(&c, Some("1")));
        assert!(!registry_matches(&c, Some("0")));
    }

    #[test]
    fn absent_value_with_default_true_counts_as_applied() {
        let c = change(
            r#"{"Path":"HKCU:\\X","Name":"A","Type":"DWord","Value":"1","OriginalValue":"0","DefaultState":"true"}"#,
        );
        assert!(registry_matches(&c, None));
    }

    #[test]
    fn absent_value_with_default_false_compares_original_to_value() {
        let c = change(TOGGLE);
        // Absent reads as OriginalValue "0", which is not the applied value "1".
        assert!(!registry_matches(&c, None));

        let same = change(
            r#"{"Path":"HKCU:\\X","Name":"A","Type":"DWord","Value":"0","OriginalValue":"0","DefaultState":"false"}"#,
        );
        assert!(registry_matches(&same, None));
    }

    #[test]
    fn absent_value_without_default_state_does_not_match() {
        let c = change(
            r#"{"Path":"HKCU:\\X","Name":"A","Type":"DWord","Value":"1","OriginalValue":"0"}"#,
        );
        assert!(!registry_matches(&c, None));
    }

    #[test]
    fn remove_entry_expects_absent_and_present_value_does_not_match() {
        let c = change(
            r#"{"Path":"HKLM:\\X","Name":"A","Type":"DWord","Value":"<RemoveEntry>","OriginalValue":"1","DefaultState":"false"}"#,
        );
        assert!(!registry_matches(&c, Some("1")));
        assert!(!registry_matches(&c, None), "original 1 is not absent");
    }

    #[test]
    fn string_values_compare_case_insensitively_and_numbers_numerically() {
        assert!(same_text(RegType::String, "Hello", "hello"));
        assert!(same_text(RegType::DWord, "01", "1"));
        assert!(!same_text(RegType::DWord, "1", "2"));
    }

    #[test]
    fn split_path_recognises_the_three_hives() {
        assert_eq!(
            split_path(r"HKLM:\SOFTWARE\Microsoft"),
            Some((Hive::LocalMachine, r"SOFTWARE\Microsoft"))
        );
        assert_eq!(split_path(r"HKCU:\Control Panel"), Some((Hive::CurrentUser, "Control Panel")));
        assert_eq!(split_path(r"HKU:\.DEFAULT\X"), Some((Hive::Users, r".DEFAULT\X")));
        assert_eq!(split_path(r"HKCR:\X"), None);
        assert_eq!(split_path("no hive"), None);
    }

    fn combo_registry() -> Vec<RegistryChange> {
        vec![
            change(
                r#"{"Path":"HKLM:\\A","Name":"One","Type":"DWord","DefaultValue":"0",
                    "Values":{"Enabled":"<RemoveEntry>","Off":"5"}}"#,
            ),
            change(
                r#"{"Path":"HKLM:\\B","Name":"Two","Type":"DWord","DefaultValue":"0",
                    "Values":{"Enabled":"<RemoveEntry>","Off":"1"}}"#,
            ),
        ]
    }

    #[test]
    fn combo_state_reads_absent_as_default_and_returns_first_matching_choice() {
        let registry = combo_registry();
        let choices = ["Enabled", "Off"];
        // Both absent: each reads as DefaultValue "0", which is what "Enabled" expects.
        assert_eq!(combo_state(&registry, &[None, None], &choices), Some("Enabled"));
        assert_eq!(
            combo_state(&registry, &[Some("5".into()), Some("1".into())], &choices),
            Some("Off")
        );
    }

    #[test]
    fn combo_state_is_none_when_no_choice_matches() {
        let registry = combo_registry();
        assert_eq!(
            combo_state(&registry, &[Some("9".into()), None], &["Enabled", "Off"]),
            None
        );
    }

    #[test]
    fn combo_writes_select_the_state_for_every_entry() {
        let writes = combo_writes(&combo_registry(), "Off").unwrap();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].value, RegValue::Set("5".into()));
        assert_eq!(writes[1].value, RegValue::Set("1".into()));
        assert!(combo_writes(&combo_registry(), "Unknown").is_none());
    }
}

//! The real Windows host, run against a throwaway `HKCU\Software\WinUtilRsTest-*` key and harmless
//! PowerShell. Nothing outside that key is written; a guard deletes it when the test ends.

use std::time::{SystemTime, UNIX_EPOCH};

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;
use winutil_core::config::registry::{RegType, RegValue};
use winutil_core::tweaks::run::{self, Outcome, TweakHost, Written, is_applied};
use winutil_core::tweaks::system::SystemTweaks;
use winutil_core::config::tweaks::Tweak;
use winutil_core::tweaks::{Direction, RegWrite};

/// A scratch key under HKCU. Dropped at the end of the test, even on panic.
struct Scratch {
    subkey: String,
    path: String,
}

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after the epoch")
            .as_nanos();
        let subkey = format!(r"Software\WinUtilRsTest-{}-{nanos}", std::process::id());
        let path = format!(r"HKCU:\{subkey}");
        Self { subkey, path }
    }

    fn write(&self, kind: RegType, name: &str, value: RegValue) -> RegWrite {
        RegWrite {
            path: self.path.clone(),
            name: name.to_owned(),
            kind,
            value,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.subkey);
    }
}

#[test]
fn each_registry_type_round_trips_through_the_real_key() {
    let scratch = Scratch::new();
    let mut host = SystemTweaks;

    let cases = [
        (RegType::DWord, "Dword", "1"),
        (RegType::QWord, "Qword", "4294967296"),
        (RegType::String, "Text", "hello"),
    ];
    for (kind, name, value) in cases {
        let write = scratch.write(kind, name, RegValue::Set(value.to_owned()));
        assert_eq!(host.write_registry(&write), Ok(Written::Set), "{name} write");
        assert_eq!(
            host.read_registry(&scratch.path, name),
            Ok(Some(value.to_owned())),
            "{name} read back"
        );
    }
}

#[test]
fn remove_deletes_the_value_then_reports_it_absent() {
    let scratch = Scratch::new();
    let mut host = SystemTweaks;

    let set = scratch.write(RegType::DWord, "Gone", RegValue::Set("7".to_owned()));
    host.write_registry(&set).expect("setup write");

    let remove = scratch.write(RegType::DWord, "Gone", RegValue::Remove);
    assert_eq!(host.write_registry(&remove), Ok(Written::Removed));
    assert_eq!(host.read_registry(&scratch.path, "Gone"), Ok(None));
    assert_eq!(host.write_registry(&remove), Ok(Written::AlreadyAbsent));
}

#[test]
fn a_service_that_does_not_exist_reads_as_none() {
    let mut host = SystemTweaks;
    assert_eq!(host.read_service("WinUtilRsNoSuchService"), Ok(None));
}

#[test]
fn a_script_that_succeeds_returns_ok() {
    let mut host = SystemTweaks;
    assert_eq!(host.run_script("Write-Output 'ok'"), Ok(()));
}

#[test]
fn a_script_that_throws_returns_an_error() {
    let mut host = SystemTweaks;
    assert!(host.run_script("throw 'boom'").is_err());
}

#[test]
fn a_toggle_applies_and_undoes_on_the_real_key() {
    let scratch = Scratch::new();
    let mut host = SystemTweaks;

    let json = serde_json::json!({
        "category": "Test",
        "Content": "Fixture",
        "link": "",
        "panel": "2",
        "Type": "Toggle",
        "registry": [{
            "Path": scratch.path,
            "Name": "Flag",
            "Type": "DWord",
            "Value": "1",
            "OriginalValue": "0",
            "DefaultState": "false",
        }],
    })
    .to_string();
    let tweak: Tweak = serde_json_lenient::from_str(&json).expect("fixture parses");

    let initial = scratch.write(RegType::DWord, "Flag", RegValue::Set("0".to_owned()));
    host.write_registry(&initial).expect("setup write");
    assert_eq!(is_applied(&mut host, &tweak), Ok(false));

    let reports = run::apply_tweak(&mut host, &tweak, Direction::Apply, true);
    assert_eq!(reports.iter().map(|r| r.outcome).collect::<Vec<_>>(), [Outcome::Done]);
    assert_eq!(host.read_registry(&scratch.path, "Flag"), Ok(Some("1".to_owned())));
    assert_eq!(is_applied(&mut host, &tweak), Ok(true));

    let reports = run::apply_tweak(&mut host, &tweak, Direction::Undo, true);
    assert_eq!(reports.iter().map(|r| r.outcome).collect::<Vec<_>>(), [Outcome::Done]);
    assert_eq!(host.read_registry(&scratch.path, "Flag"), Ok(Some("0".to_owned())));
    assert_eq!(is_applied(&mut host, &tweak), Ok(false));
}

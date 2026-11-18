//! Tweak runner behaviour against an in-memory [`TweakHost`]. Covers each action kind: registry
//! toggles, value removal, services, scripts, combobox choices with rollback, and buttons.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use winutil_core::config::registry::RegValue;
use winutil_core::config::tweaks::{StartupType, Tweak};
use winutil_core::tweaks::run::{self, Action, OOSU_URL, Outcome, Report, TweakHost, Written};
use winutil_core::tweaks::{Direction, RegWrite};

const REG_PATH: &str = r"HKCU:\Software\WinUtilRsFixture";
const COMBO_PATH: &str = r"HKLM:\SOFTWARE\WinUtilRsFixture";
const FLAG: &str = r#""registry":[{"Path":"HKCU:\\Software\\WinUtilRsFixture","Name":"Flag","Type":"DWord","Value":"1","OriginalValue":"0","DefaultState":"false"}]"#;
const COMBO: &str = r#""registry":[{"Path":"HKLM:\\SOFTWARE\\WinUtilRsFixture","Name":"One","Type":"DWord","DefaultValue":"0","Values":{"Enabled":"<RemoveEntry>","Off":"5"}},{"Path":"HKLM:\\SOFTWARE\\WinUtilRsFixture","Name":"Two","Type":"DWord","DefaultValue":"0","Values":{"Enabled":"<RemoveEntry>","Off":"1"}}],"ComboItems":"Enabled|Off""#;
const SERVICE: &str = r#""service":[{"Name":"svc","StartupType":"Disabled","OriginalType":"Manual"}]"#;

/// Builds a tweak from the JSON members that follow the fixed fields. `members` must be non-empty.
fn tweak(members: &str) -> Tweak {
    let json = format!(r#"{{"category":"Test","Content":"Fixture","link":"","panel":"2",{members}}}"#);
    serde_json_lenient::from_str(&json).expect("fixture parses")
}

fn outcomes(reports: &[Report]) -> Vec<Outcome> {
    reports.iter().map(|r| r.outcome).collect()
}

/// In-memory machine. Every method records what it was asked to do.
#[derive(Default)]
struct Fake {
    values: HashMap<(String, String), String>,
    services: HashMap<String, StartupType>,
    service_calls: Vec<(String, StartupType)>,
    scripts: Vec<String>,
    failing_script: Option<String>,
    /// Writes to this value name always fail.
    fail_writes_to: Option<String>,
    /// The next write to this value name fails once.
    fail_first_write_to: Option<String>,
    /// Writes to this value name report success but store nothing.
    drop_writes_to: Option<String>,
    programs: Vec<(String, Vec<String>)>,
    program_replies: VecDeque<Option<(i32, String)>>,
    download_error: Option<String>,
    downloads: Vec<(String, PathBuf)>,
    launched: Vec<PathBuf>,
}

impl Fake {
    fn with_value(mut self, path: &str, name: &str, text: &str) -> Self {
        self.values
            .insert((path.to_owned(), name.to_owned()), text.to_owned());
        self
    }

    fn value(&self, path: &str, name: &str) -> Option<&str> {
        self.values
            .get(&(path.to_owned(), name.to_owned()))
            .map(String::as_str)
    }
}

impl TweakHost for Fake {
    fn read_registry(&mut self, path: &str, name: &str) -> Result<Option<String>, String> {
        Ok(self.value(path, name).map(str::to_owned))
    }

    fn write_registry(&mut self, write: &RegWrite) -> Result<Written, String> {
        let name = write.name.as_str();
        if self.fail_writes_to.as_deref() == Some(name) {
            return Err("access denied".into());
        }
        if self.fail_first_write_to.as_deref() == Some(name) {
            self.fail_first_write_to = None;
            return Err("access denied".into());
        }
        if self.drop_writes_to.as_deref() == Some(name) {
            return Ok(Written::Set);
        }
        let key = (write.path.clone(), write.name.clone());
        match &write.value {
            RegValue::Remove => Ok(if self.values.remove(&key).is_some() {
                Written::Removed
            } else {
                Written::AlreadyAbsent
            }),
            RegValue::Set(text) => {
                self.values.insert(key, text.clone());
                Ok(Written::Set)
            }
        }
    }

    fn read_service(&mut self, name: &str) -> Result<Option<StartupType>, String> {
        Ok(self.services.get(name).copied())
    }

    fn set_service(&mut self, name: &str, target: StartupType) -> Result<(), String> {
        self.service_calls.push((name.to_owned(), target));
        self.services.insert(name.to_owned(), target);
        Ok(())
    }

    fn run_script(&mut self, script: &str) -> Result<(), String> {
        self.scripts.push(script.to_owned());
        if self.failing_script.as_deref() == Some(script) {
            Err("exit code 1: boom".into())
        } else {
            Ok(())
        }
    }

    fn run_program(&mut self, program: &str, args: &[&str]) -> Option<(i32, String)> {
        self.programs.push((
            program.to_owned(),
            args.iter().map(|a| (*a).to_owned()).collect(),
        ));
        self.program_replies.pop_front().flatten()
    }

    fn download(&mut self, url: &str, dest: &Path) -> Result<(), String> {
        self.downloads.push((url.to_owned(), dest.to_path_buf()));
        match &self.download_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn launch(&mut self, program: &Path) -> Result<(), String> {
        self.launched.push(program.to_path_buf());
        Ok(())
    }

    fn winutil_dir(&self) -> Result<PathBuf, String> {
        Ok(PathBuf::from("fake-winutil"))
    }
}

#[test]
fn toggle_apply_then_undo_is_reflected_by_the_status_check() {
    let tweak = tweak(FLAG);
    let mut host = Fake::default();
    assert!(!run::is_applied(&mut host, &tweak).unwrap(), "absent with original 0 is not applied");

    let reports = run::run(&mut host, "Flag", &tweak, Action::Tweak(Direction::Apply));
    assert_eq!(outcomes(&reports), [Outcome::Done]);
    assert_eq!(host.value(REG_PATH, "Flag"), Some("1"));
    assert!(run::is_applied(&mut host, &tweak).unwrap());

    let reports = run::run(&mut host, "Flag", &tweak, Action::Tweak(Direction::Undo));
    assert_eq!(outcomes(&reports), [Outcome::Done]);
    assert_eq!(host.value(REG_PATH, "Flag"), Some("0"), "undo writes OriginalValue");
    assert!(!run::is_applied(&mut host, &tweak).unwrap());
}

#[test]
fn remove_entry_deletes_the_value_then_reports_already_absent() {
    let tweak = tweak(
        r#""registry":[{"Path":"HKCU:\\Software\\WinUtilRsFixture","Name":"Flag","Type":"DWord","Value":"<RemoveEntry>","OriginalValue":"1","DefaultState":"true"}]"#,
    );
    let mut host = Fake::default().with_value(REG_PATH, "Flag", "1");

    let first = run::run(&mut host, "Flag", &tweak, Action::Tweak(Direction::Apply));
    assert_eq!(outcomes(&first), [Outcome::Done]);
    assert_eq!(host.value(REG_PATH, "Flag"), None);

    let second = run::run(&mut host, "Flag", &tweak, Action::Tweak(Direction::Apply));
    assert_eq!(outcomes(&second), [Outcome::Skipped]);
    assert_eq!(second[0].detail, format!(r"{REG_PATH}\Flag is already absent"));
}

#[test]
fn registry_write_failure_is_reported_and_later_steps_still_run() {
    let tweak = tweak(&format!(r#"{FLAG},"InvokeScript":["after"]"#));
    let mut host = Fake {
        fail_writes_to: Some("Flag".into()),
        ..Fake::default()
    };

    let reports = run::run(&mut host, "Flag", &tweak, Action::Tweak(Direction::Apply));
    assert_eq!(outcomes(&reports), [Outcome::Failed, Outcome::Done]);
    assert_eq!(
        reports[0].detail,
        format!(r"could not change {REG_PATH}\Flag: access denied")
    );
    assert_eq!(host.scripts, ["after"]);
}

#[test]
fn service_is_set_when_it_is_in_the_expected_state() {
    let tweak = tweak(SERVICE);
    let mut host = Fake::default();
    host.services.insert("svc".into(), StartupType::Manual);

    let reports = run::run(&mut host, "Svc", &tweak, Action::Tweak(Direction::Apply));
    assert_eq!(outcomes(&reports), [Outcome::Done]);
    assert_eq!(host.service_calls, [("svc".to_owned(), StartupType::Disabled)]);
}

#[test]
fn service_in_unexpected_state_is_left_alone_while_keep_startup_is_on() {
    let tweak = tweak(SERVICE);
    let mut host = Fake::default();
    host.services.insert("svc".into(), StartupType::Automatic);

    let reports = run::apply_tweak(&mut host, &tweak, Direction::Apply, true);
    assert_eq!(outcomes(&reports), [Outcome::Skipped]);
    assert_eq!(
        reports[0].detail,
        "Service svc startup type is Automatic, not Manual as this tweak expects; leaving it unchanged."
    );
    assert!(host.service_calls.is_empty());
}

#[test]
fn service_in_unexpected_state_is_overwritten_when_keep_startup_is_off() {
    let tweak = tweak(SERVICE);
    let mut host = Fake::default();
    host.services.insert("svc".into(), StartupType::Automatic);

    let reports = run::apply_tweak(&mut host, &tweak, Direction::Apply, false);
    assert_eq!(outcomes(&reports), [Outcome::Done]);
    assert_eq!(host.service_calls, [("svc".to_owned(), StartupType::Disabled)]);
}

#[test]
fn service_already_at_target_is_skipped_without_a_write() {
    let tweak = tweak(SERVICE);
    let mut host = Fake::default();
    host.services.insert("svc".into(), StartupType::Disabled);

    let reports = run::apply_tweak(&mut host, &tweak, Direction::Apply, false);
    assert_eq!(outcomes(&reports), [Outcome::Skipped]);
    assert_eq!(reports[0].detail, "service svc is already Disabled");
    assert!(host.service_calls.is_empty());
}

#[test]
fn missing_service_is_skipped_and_later_steps_still_run() {
    let tweak = tweak(&format!(r#"{SERVICE},"InvokeScript":["after"]"#));
    let mut host = Fake::default();

    let reports = run::apply_tweak(&mut host, &tweak, Direction::Apply, true);
    assert_eq!(outcomes(&reports), [Outcome::Skipped, Outcome::Done]);
    assert_eq!(reports[0].detail, "Service svc was not found.");
    assert_eq!(host.scripts, ["after"]);
}

#[test]
fn undo_runs_undo_scripts_and_apply_runs_invoke_scripts_in_order() {
    let tweak = tweak(r#""InvokeScript":["a","b"],"UndoScript":["c"]"#);
    let mut host = Fake::default();

    let applied = run::apply_tweak(&mut host, &tweak, Direction::Apply, true);
    assert_eq!(outcomes(&applied), [Outcome::Done, Outcome::Done]);
    let undone = run::apply_tweak(&mut host, &tweak, Direction::Undo, true);
    assert_eq!(outcomes(&undone), [Outcome::Done]);
    assert_eq!(host.scripts, ["a", "b", "c"]);
}

#[test]
fn failing_script_is_reported_and_the_next_script_still_runs() {
    let tweak = tweak(r#""InvokeScript":["a","b","c"]"#);
    let mut host = Fake {
        failing_script: Some("b".into()),
        ..Fake::default()
    };

    let reports = run::apply_tweak(&mut host, &tweak, Direction::Apply, true);
    assert_eq!(
        outcomes(&reports),
        [Outcome::Done, Outcome::Failed, Outcome::Done]
    );
    assert_eq!(reports[1].detail, "PowerShell script failed: exit code 1: boom");
    assert_eq!(host.scripts, ["a", "b", "c"]);
}

#[test]
fn combo_choice_writes_every_entry_and_reads_back() {
    let tweak = tweak(COMBO);
    let mut host = Fake::default();

    let reports = run::run(&mut host, "Multi", &tweak, Action::Choice("Off"));
    assert_eq!(outcomes(&reports), [Outcome::Done]);
    assert_eq!(host.value(COMBO_PATH, "One"), Some("5"));
    assert_eq!(host.value(COMBO_PATH, "Two"), Some("1"));
    assert_eq!(
        run::current_choice(&mut host, &tweak).unwrap().as_deref(),
        Some("Off")
    );
}

#[test]
fn combo_failed_write_restores_the_previous_value() {
    let tweak = tweak(COMBO);
    let mut host = Fake {
        fail_first_write_to: Some("Two".into()),
        ..Fake::default()
    }
    .with_value(COMBO_PATH, "One", "0");

    let reports = run::run(&mut host, "Multi", &tweak, Action::Choice("Off"));
    assert_eq!(outcomes(&reports), [Outcome::Failed]);
    assert_eq!(reports[0].detail, "Unable to apply registry state 'Off': access denied");
    assert_eq!(host.value(COMBO_PATH, "One"), Some("0"), "written entry is restored");
    assert_eq!(host.value(COMBO_PATH, "Two"), None, "never-written entry stays absent");
}

#[test]
fn combo_failed_write_removes_entries_that_were_absent_before() {
    let tweak = tweak(COMBO);
    let mut host = Fake {
        fail_first_write_to: Some("Two".into()),
        ..Fake::default()
    };

    let reports = run::run(&mut host, "Multi", &tweak, Action::Choice("Off"));
    assert_eq!(outcomes(&reports), [Outcome::Failed]);
    assert_eq!(host.value(COMBO_PATH, "One"), None);
}

#[test]
fn combo_restore_failure_is_appended_to_the_message() {
    let tweak = tweak(COMBO);
    let mut host = Fake {
        fail_writes_to: Some("Two".into()),
        ..Fake::default()
    };

    let reports = run::run(&mut host, "Multi", &tweak, Action::Choice("Off"));
    assert_eq!(
        reports[0].detail,
        "Unable to apply registry state 'Off': access denied The previous registry state could not be restored: access denied"
    );
}

#[test]
fn combo_read_back_mismatch_rolls_back_and_reports() {
    let tweak = tweak(COMBO);
    let mut host = Fake {
        drop_writes_to: Some("Two".into()),
        ..Fake::default()
    };

    let reports = run::run(&mut host, "Multi", &tweak, Action::Choice("Off"));
    assert_eq!(outcomes(&reports), [Outcome::Failed]);
    assert_eq!(
        reports[0].detail,
        "Unable to apply registry state 'Off': the stored values do not match the choice after writing"
    );
    assert_eq!(host.value(COMBO_PATH, "One"), None);
}

#[test]
fn unknown_combo_choice_is_rejected_without_writing() {
    let tweak = tweak(COMBO);
    let mut host = Fake::default();

    let reports = run::run(&mut host, "Multi", &tweak, Action::Choice("Nope"));
    assert_eq!(outcomes(&reports), [Outcome::Failed]);
    assert_eq!(reports[0].detail, "'Nope' is not a choice for this tweak.");
    assert!(host.values.is_empty());
}

#[test]
fn current_combo_choice_is_none_when_no_choice_matches() {
    let tweak = tweak(COMBO);
    let mut host = Fake::default().with_value(COMBO_PATH, "One", "9");
    assert_eq!(run::current_choice(&mut host, &tweak).unwrap(), None);
}

#[test]
fn tweak_without_registry_entries_counts_as_applied() {
    let tweak = tweak(r#""Type":"Button""#);
    let mut host = Fake::default();
    assert!(run::is_applied(&mut host, &tweak).unwrap());
}

#[test]
fn oosu_button_downloads_then_launches_the_tool() {
    let tweak = tweak(r#""Type":"Button""#);
    let mut host = Fake::default();

    let reports = run::run(&mut host, "WPFOOSUbutton", &tweak, Action::Button);
    assert_eq!(outcomes(&reports), [Outcome::Done]);
    let dest = PathBuf::from("fake-winutil").join("ooshutup10.exe");
    assert_eq!(host.downloads, [(OOSU_URL.to_owned(), dest.clone())]);
    assert_eq!(host.launched, [dest]);
}

#[test]
fn oosu_download_failure_is_reported_and_the_tool_is_not_launched() {
    let tweak = tweak(r#""Type":"Button""#);
    let mut host = Fake {
        download_error: Some("network down".into()),
        ..Fake::default()
    };

    let reports = run::run(&mut host, "WPFOOSUbutton", &tweak, Action::Button);
    assert_eq!(outcomes(&reports), [Outcome::Failed]);
    assert_eq!(reports[0].detail, "network down");
    assert!(host.launched.is_empty());
}

#[test]
fn ultimate_performance_button_duplicates_then_activates() {
    let tweak = tweak(r#""Type":"Button""#);
    let mut host = Fake {
        program_replies: VecDeque::from([
            Some((
                0,
                "Power Scheme GUID: 12345678-1234-4234-8234-123456789abc  (Ultimate Performance)"
                    .to_owned(),
            )),
            Some((0, String::new())),
        ]),
        ..Fake::default()
    };

    let reports = run::run(&mut host, "WPFAddUltPerf", &tweak, Action::Button);
    assert_eq!(outcomes(&reports), [Outcome::Done]);
    assert_eq!(host.programs[1].0, "powercfg");
    assert_eq!(
        host.programs[1].1,
        ["/setactive", "12345678-1234-4234-8234-123456789abc"]
    );
}

#[test]
fn remove_ultimate_performance_button_reports_powercfg_exit_code() {
    let tweak = tweak(r#""Type":"Button""#);
    let mut host = Fake {
        program_replies: VecDeque::from([Some((1, String::new()))]),
        ..Fake::default()
    };

    let reports = run::run(&mut host, "WPFRemoveUltPerf", &tweak, Action::Button);
    assert_eq!(outcomes(&reports), [Outcome::Failed]);
    assert_eq!(
        reports[0].detail,
        "powercfg could not restore the default power schemes (exit code 1)."
    );
}

#[test]
fn unknown_button_is_reported_as_failed() {
    let tweak = tweak(r#""Type":"Button""#);
    let mut host = Fake::default();

    let reports = run::run(&mut host, "WPFNope", &tweak, Action::Button);
    assert_eq!(outcomes(&reports), [Outcome::Failed]);
    assert_eq!(reports[0].detail, "No action is defined for the button WPFNope.");
}

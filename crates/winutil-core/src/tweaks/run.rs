//! Runs tweaks against a [`TweakHost`]: registry writes, service changes, PowerShell scripts,
//! combobox choices, and the button actions. Every step produces a [`Report`], so one failed step
//! does not stop the rest.

use std::path::Path;

use super::power;
use super::{
    Direction, KEEP_SERVICE_STARTUP, RegWrite, ServiceAction, ServiceWrite, combo_state,
    combo_writes, plan, registry_matches, service_action,
};
use crate::config::registry::{RegValue, RegistryChange};
use crate::config::tweaks::{StartupType, Tweak};

/// The OOSU10 download the "O&O ShutUp10" button fetches.
pub const OOSU_URL: &str = "https://dl5.oo-software.com/files/ooshutup10/OOSU10.exe";
const OOSU_FILE: &str = "ooshutup10.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    pub detail: String,
}

impl Report {
    fn done(detail: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Done,
            detail: detail.into(),
        }
    }

    fn skipped(detail: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Skipped,
            detail: detail.into(),
        }
    }

    fn failed(detail: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Failed,
            detail: detail.into(),
        }
    }
}

/// What a registry write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    Set,
    Removed,
    AlreadyAbsent,
}

/// Everything a tweak can touch on the machine. The real implementation is
/// [`super::system::SystemTweaks`]; tests use an in-memory fake.
pub trait TweakHost {
    /// The stored value as text (decimal for numbers). `None` when the key or the value is absent.
    fn read_registry(&mut self, path: &str, name: &str) -> Result<Option<String>, String>;
    fn write_registry(&mut self, write: &RegWrite) -> Result<Written, String>;
    /// `None` when the service does not exist.
    fn read_service(&mut self, name: &str) -> Result<Option<StartupType>, String>;
    fn set_service(&mut self, name: &str, target: StartupType) -> Result<(), String>;
    fn run_script(&mut self, script: &str) -> Result<(), String>;
    /// Exit code and output, or `None` if the program could not start.
    fn run_program(&mut self, program: &str, args: &[&str]) -> Option<(i32, String)>;
    fn download(&mut self, url: &str, dest: &Path) -> Result<(), String>;
    fn launch(&mut self, program: &Path) -> Result<(), String>;
    /// The per-user WinUtil directory, where downloaded tools are kept.
    fn winutil_dir(&self) -> Result<std::path::PathBuf, String>;
}

/// What the UI asks a tweak to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action<'a> {
    /// Apply or undo the registry, service, and script steps.
    Tweak(Direction),
    /// Select a combobox choice.
    Choice(&'a str),
    /// Press a button tweak. The key is the tweak's content name, e.g. `WPFOOSUbutton`.
    Button,
}

/// Runs `action` for `tweak`, returning one report per step.
pub fn run(host: &mut impl TweakHost, key: &str, tweak: &Tweak, action: Action<'_>) -> Vec<Report> {
    match action {
        Action::Tweak(direction) => apply_tweak(host, tweak, direction, KEEP_SERVICE_STARTUP),
        Action::Choice(state) => vec![choice(host, tweak, state)],
        Action::Button => vec![button(host, key)],
    }
}

/// Runs the registry, service, and script steps of `tweak` in that order.
pub fn apply_tweak(
    host: &mut impl TweakHost,
    tweak: &Tweak,
    direction: Direction,
    keep_startup: bool,
) -> Vec<Report> {
    let plan = plan(tweak, direction);
    let mut reports = Vec::new();
    for write in &plan.registry {
        let result = host.write_registry(write);
        reports.push(registry_report(result, write));
    }
    for write in &plan.services {
        reports.push(service_step(host, write, keep_startup));
    }
    for script in &plan.scripts {
        reports.push(match host.run_script(script) {
            Ok(()) => Report::done("PowerShell script ran"),
            Err(error) => Report::failed(format!("PowerShell script failed: {error}")),
        });
    }
    reports
}

fn registry_report(result: Result<Written, String>, write: &RegWrite) -> Report {
    let target = format!("{}\\{}", write.path, write.name);
    match result {
        Ok(Written::Set) => Report::done(format!("set {target}")),
        Ok(Written::Removed) => Report::done(format!("removed {target}")),
        Ok(Written::AlreadyAbsent) => Report::skipped(format!("{target} is already absent")),
        Err(error) => Report::failed(format!("could not change {target}: {error}")),
    }
}

fn service_step(host: &mut impl TweakHost, write: &ServiceWrite, keep: bool) -> Report {
    let name = &write.name;
    let current = match host.read_service(name) {
        Ok(current) => current,
        Err(error) => return Report::failed(format!("could not read service {name}: {error}")),
    };
    match service_action(write, current, keep) {
        ServiceAction::Set(target) => match host.set_service(name, target) {
            Ok(()) => Report::done(format!("set service {name} to {target:?}")),
            Err(error) => Report::failed(format!("could not set service {name}: {error}")),
        },
        ServiceAction::Unchanged => {
            Report::skipped(format!("service {name} is already {:?}", write.target))
        }
        ServiceAction::Missing => Report::skipped(format!("Service {name} was not found.")),
        ServiceAction::Mismatch(current) => Report::skipped(format!(
            "Service {name} startup type is {current:?}, not {:?} as this tweak expects; leaving it unchanged.",
            write.expected
        )),
    }
}

/// Reads every registry entry of `registry` in order.
fn read_all(
    host: &mut impl TweakHost,
    registry: &[RegistryChange],
) -> Result<Vec<Option<String>>, String> {
    registry
        .iter()
        .map(|change| host.read_registry(&change.path, &change.name))
        .collect()
}

/// Whether every registry entry of `tweak` is in its applied state. A tweak with no registry
/// entries counts as applied, as in the original.
pub fn is_applied(host: &mut impl TweakHost, tweak: &Tweak) -> Result<bool, String> {
    for change in &tweak.registry {
        let current = host.read_registry(&change.path, &change.name)?;
        if !registry_matches(change, current.as_deref()) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The combobox choice the registry currently matches, or `None` if it matches none.
pub fn current_choice(host: &mut impl TweakHost, tweak: &Tweak) -> Result<Option<String>, String> {
    let current = read_all(host, &tweak.registry)?;
    let choices = tweak.combo_choices();
    Ok(combo_state(&tweak.registry, &current, &choices).map(str::to_owned))
}

fn choice(host: &mut impl TweakHost, tweak: &Tweak, state: &str) -> Report {
    match apply_choice(host, tweak, state) {
        Ok(()) => Report::done(format!("registry set to '{state}'")),
        Err(error) => Report::failed(error),
    }
}

/// Writes the values for `state`, then reads them back. If any write fails or the read-back does
/// not match, every entry is restored to the value it had before.
fn apply_choice(host: &mut impl TweakHost, tweak: &Tweak, state: &str) -> Result<(), String> {
    let writes = combo_writes(&tweak.registry, state)
        .ok_or_else(|| format!("'{state}' is not a choice for this tweak."))?;
    let previous = writes
        .iter()
        .map(|write| host.read_registry(&write.path, &write.name))
        .collect::<Result<Vec<_>, _>>()?;

    let attempt = write_all(host, &writes).and_then(|()| {
        let current = read_all(host, &tweak.registry)?;
        let choices = tweak.combo_choices();
        match combo_state(&tweak.registry, &current, &choices) {
            Some(found) if found == state => Ok(()),
            _ => Err("the stored values do not match the choice after writing".to_owned()),
        }
    });

    let Err(error) = attempt else {
        return Ok(());
    };
    let message = format!("Unable to apply registry state '{state}': {error}");
    match restore(host, &writes, &previous) {
        Ok(()) => Err(message),
        Err(restore_error) => Err(format!(
            "{message} The previous registry state could not be restored: {restore_error}"
        )),
    }
}

fn write_all(host: &mut impl TweakHost, writes: &[RegWrite]) -> Result<(), String> {
    for write in writes {
        host.write_registry(write)?;
    }
    Ok(())
}

/// Puts back the values in `previous`: `Some` is written back, `None` (absent) is removed.
fn restore(
    host: &mut impl TweakHost,
    writes: &[RegWrite],
    previous: &[Option<String>],
) -> Result<(), String> {
    for (write, prior) in writes.iter().zip(previous) {
        let value = match prior {
            Some(text) => RegValue::Set(text.clone()),
            None => RegValue::Remove,
        };
        host.write_registry(&RegWrite {
            path: write.path.clone(),
            name: write.name.clone(),
            kind: write.kind,
            value,
        })?;
    }
    Ok(())
}

fn button(host: &mut impl TweakHost, key: &str) -> Report {
    let result = match key {
        "WPFOOSUbutton" => oosu(host),
        "WPFAddUltPerf" => power::enable(&mut |program, args| host.run_program(program, args))
            .map(|()| "Ultimate Performance power plan is active.".to_owned()),
        "WPFRemoveUltPerf" => power::disable(&mut |program, args| host.run_program(program, args))
            .map(|()| "Default power schemes restored.".to_owned()),
        other => Err(format!("No action is defined for the button {other}.")),
    };
    match result {
        Ok(detail) => Report::done(detail),
        Err(error) => Report::failed(error),
    }
}

fn oosu(host: &mut impl TweakHost) -> Result<String, String> {
    let dest = host.winutil_dir()?.join(OOSU_FILE);
    host.download(OOSU_URL, &dest)?;
    host.launch(&dest)?;
    Ok(format!("Started {}.", dest.display()))
}


//! Real [`FeatureHost`]: optional features through `dism.exe`, scripts through
//! [`crate::powershell`].
//!
//! `dism.exe` runs directly instead of `Enable-WindowsOptionalFeature`. The original runs all
//! features in one PowerShell session, so there is no per-feature startup to remove. Measured in
//! docs/features-perf.md: one DISM process costs more per feature than the cmdlet inside an
//! existing session, but less than a standalone PowerShell script per entry. Exit code 3010
//! (reboot pending) is success, as with `-NoRestart`.

use super::FeatureHost;
use crate::install::run::{last_line, run_command};
use crate::powershell;

/// DISM exit code for success with a reboot still pending. `-NoRestart` leaves the reboot to the
/// user, so this is success.
const REBOOT_REQUIRED: i32 = 3010;

/// The production host. Stateless: each call talks to the machine directly.
pub struct SystemFeatures;

impl FeatureHost for SystemFeatures {
    fn enable(&mut self, name: &str) -> Result<(), String> {
        let feature_arg = format!("/featurename:{name}");
        let args = [
            "/online",
            "/enable-feature",
            feature_arg.as_str(),
            "/all",
            "/norestart",
        ];
        let (code, text) =
            run_command("dism.exe", &args).ok_or_else(|| "DISM could not be started".to_owned())?;
        dism_result(code, &text)
    }

    fn run_script(&mut self, script: &str) -> Result<(), String> {
        powershell::run_script(script)
    }
}

/// Maps a DISM exit code to success or the last line of its output.
pub fn dism_result(code: i32, output: &str) -> Result<(), String> {
    match code {
        0 | REBOOT_REQUIRED => Ok(()),
        _ => Err(format!("DISM exit code {code}: {}", last_line(output))),
    }
}

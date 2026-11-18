//! The Explorer helper that tweak scripts call by name must exist when the production host runs
//! a script. The probe only checks that the function is defined; it never calls it, so nothing
//! on the machine changes.

use winutil_core::tweaks::run::TweakHost;
use winutil_core::tweaks::system::SystemTweaks;

#[test]
fn scripts_run_with_the_explorer_helper_defined() {
    let script = r#"
if ((Get-Command Invoke-WinUtilExplorerUpdate -ErrorAction SilentlyContinue).CommandType -ne 'Function') {
    exit 3
}
"#;
    let result = SystemTweaks.run_script(script);
    assert_eq!(result, Ok(()));
}

#[test]
fn a_script_failure_still_reports_its_exit_code() {
    let result = SystemTweaks.run_script("exit 5");
    let error = result.expect_err("exit 5 must be reported as a failure");
    assert!(error.contains("exit code 5"), "unexpected error: {error}");
}

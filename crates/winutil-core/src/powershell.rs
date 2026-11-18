//! Runs PowerShell scripts for the Tweaks and Features tabs.
//!
//! Every script gets the same prelude first, so a script can call a helper by the name the
//! original used.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::install::run::last_line;

/// Helpers that scripts call by name. `Invoke-WinUtilExplorerUpdate` is copied from the
/// original's `functions/private`; its `Invoke-WPFRunspace` wrapper is dropped because the call
/// here is already synchronous.
const PRELUDE: &str = r#"
function Invoke-WinUtilExplorerUpdate {
    param (
        [string]$action = "refresh"
    )

    if ($action -eq "refresh") {
        if (-not ([System.Management.Automation.PSTypeName]'Win32').Type) {
            Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class Win32 {
    [DllImport("user32.dll", CharSet = CharSet.Auto, SetLastError = false)]
    public static extern IntPtr SendMessageTimeout(
        IntPtr hWnd, uint Msg, IntPtr wParam, string lParam,
        uint fuFlags, uint uTimeout, out IntPtr lpdwResult);
}
"@
        }

        $HWND_BROADCAST = [IntPtr]0xffff
        $WM_SETTINGCHANGE = 0x1A
        $SMTO_ABORTIFHUNG = 0x2

        [Win32]::SendMessageTimeout($HWND_BROADCAST, $WM_SETTINGCHANGE,
            [IntPtr]::Zero, "ImmersiveColorSet", $SMTO_ABORTIFHUNG, 100,
            [ref]([IntPtr]::Zero))
    } elseif ($action -eq "restart") {
        taskkill.exe /F /IM "explorer.exe"
        Start-Process "explorer.exe"
    }
}
"#;

/// The text sent to PowerShell: the helpers, then the script.
pub fn powershell_input(script: &str) -> String {
    format!("{PRELUDE}\n{script}")
}

/// Runs `script` in a new `powershell.exe`. The script goes in on stdin, so nothing needs quoting
/// on the command line. A non-zero exit is an error, reported with the last line of stderr.
pub fn run_script(script: &str) -> Result<(), String> {
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("PowerShell could not be started: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(powershell_input(script).as_bytes())
            .map_err(|e| format!("could not send the script to PowerShell: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("PowerShell did not finish: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let code = output.status.code().unwrap_or(-1);
    let text = String::from_utf8_lossy(&output.stderr);
    Err(format!("exit code {code}: {}", last_line(&text)))
}

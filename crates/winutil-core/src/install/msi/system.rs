//! Real [`MsiHost`]: registry, `pwsh.exe`, WinGet, the GitHub API, download, and `msiexec`.
//!
//! Only the glue lives here. The decisions it feeds are tested in the parent module.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

use super::{MsiHost, Release, digest_matches, is_stable_pwsh7, pins_allow_upgrade, signature_trusted};
use crate::install::find_on_path;
use crate::install::run::run_command;
use crate::net::{self, agent};

const UNINSTALL_KEYS: [&str; 2] = [
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
];
const CORE_VERSION_KEYS: [&str; 2] = [
    r"SOFTWARE\Microsoft\PowerShellCore\InstalledVersions",
    r"SOFTWARE\WOW6432Node\Microsoft\PowerShellCore\InstalledVersions",
];
const RELEASE_URL: &str = "https://api.github.com/repos/PowerShell/PowerShell/releases/latest";
const API_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The production host. Stateless: each call talks to the machine directly.
pub struct SystemMsi;

impl MsiHost for SystemMsi {
    fn msi_installs(&mut self) -> Vec<PathBuf> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let mut found: Vec<PathBuf> = Vec::new();
        for key in UNINSTALL_KEYS {
            let Ok(uninstall) = hklm.open_subkey(key) else { continue };
            for product_code in uninstall.enum_keys().flatten() {
                let Ok(entry) = uninstall.open_subkey(&product_code) else { continue };
                let windows_installer: u32 = entry.get_value("WindowsInstaller").unwrap_or(0);
                if windows_installer != 1 {
                    continue;
                }
                let display_name: String = entry.get_value("DisplayName").unwrap_or_default();
                if !is_stable_pwsh7(&display_name) {
                    continue;
                }
                let location: String = entry.get_value("InstallLocation").unwrap_or_default();
                let location = if location.trim().is_empty() {
                    core_install_location(&hklm, &product_code)
                } else {
                    location
                };
                let dir = PathBuf::from(location.trim().trim_end_matches(['\\', '/']));
                if dir.as_os_str().is_empty() || !dir.join("pwsh.exe").exists() {
                    continue;
                }
                if !found.iter().any(|known| known.as_os_str().eq_ignore_ascii_case(dir.as_os_str())) {
                    found.push(dir);
                }
            }
        }
        found
    }

    fn standalone_pwsh(&mut self) -> Option<PathBuf> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        find_on_path("pwsh", &path)
    }

    fn installed_version(&mut self, pwsh: &Path) -> Result<String, String> {
        pwsh_output(pwsh, "$PSVersionTable.PSVersion.ToString()")
    }

    fn architecture(&mut self, pwsh: &Path) -> Result<String, String> {
        pwsh_output(
            pwsh,
            "[System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()",
        )
    }

    fn upgrade_allowed(&mut self) -> bool {
        let args = ["pin", "list", "--accept-source-agreements", "--disable-interactivity"];
        match run_command("winget", &args) {
            Some((code, output)) => pins_allow_upgrade(&output, code == 0),
            None => false,
        }
    }

    fn latest_release(&mut self) -> Result<Release, String> {
        let agent = agent(API_TIMEOUT);
        let mut response = agent
            .get(RELEASE_URL)
            .header("User-Agent", "winutil-rs")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| format!("GitHub release lookup failed: {error}"))?;
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("GitHub release lookup failed: {error}"))?;
        serde_json::from_str(&body).map_err(|error| format!("GitHub release JSON was invalid: {error}"))
    }

    fn scratch_path(&mut self, extension: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!("PowerShell-{nanos:x}-{}.{extension}", std::process::id()))
    }

    fn download(&mut self, url: &str, dest: &Path) -> Result<(), String> {
        net::download(url, dest, DOWNLOAD_TIMEOUT)
    }

    fn verify(&mut self, msi: &Path, digest: Option<&str>) -> Result<(), String> {
        if let Some(expected) = digest {
            let actual = sha256_file(msi)?;
            if !digest_matches(expected, &actual) {
                return Err("PowerShell MSI SHA256 verification failed".to_owned());
            }
        }
        let quoted = msi.display().to_string().replace('\'', "''");
        let script = format!(
            "$s = Get-AuthenticodeSignature -LiteralPath '{quoted}'; \
             Write-Output $s.Status; Write-Output $s.SignerCertificate.Subject"
        );
        let args = ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script.as_str()];
        let (_, output) = run_command("powershell.exe", &args)
            .ok_or_else(|| "could not start powershell.exe to check the signature".to_owned())?;
        let mut lines = output.lines();
        let status = lines.next().unwrap_or_default().trim();
        let subject = lines.next().map(str::trim).filter(|line| !line.is_empty());
        if signature_trusted(status, subject) {
            Ok(())
        } else {
            Err("PowerShell MSI signature verification failed".to_owned())
        }
    }

    fn install(&mut self, msi: &Path) -> Result<i32, String> {
        let path = msi.display().to_string();
        let args = ["/i", path.as_str(), "/qn", "/norestart"];
        run_command("msiexec.exe", &args)
            .map(|(code, _)| code)
            .ok_or_else(|| "msiexec did not start".to_owned())
    }

    fn install_msix(&mut self, bundle: &Path) -> Result<(), String> {
        let quoted = bundle.display().to_string().replace('\'', "''");
        let script = format!("Add-AppxPackage -Path '{quoted}' -ErrorAction Stop");
        let args = ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script.as_str()];
        let (code, _) = run_command("powershell.exe", &args)
            .ok_or_else(|| "could not start powershell.exe to install the MSIX bundle".to_owned())?;
        if code == 0 {
            Ok(())
        } else {
            Err(format!("MSIX bundle installation failed with exit code {code}"))
        }
    }

    fn remove(&mut self, path: &Path) {
        let _ = fs::remove_file(path);
    }
}

/// Install location from the PowerShellCore registration whose `ProductCode` matches the MSI key.
/// Empty when no registration matches, mirroring the original's `Select -First 1`.
fn core_install_location(hklm: &RegKey, product_code: &str) -> String {
    for key in CORE_VERSION_KEYS {
        let Ok(versions) = hklm.open_subkey(key) else { continue };
        for version in versions.enum_keys().flatten() {
            let Ok(entry) = versions.open_subkey(&version) else { continue };
            let code: String = entry.get_value("ProductCode").unwrap_or_default();
            if code.eq_ignore_ascii_case(product_code) {
                return entry.get_value("InstallLocation").unwrap_or_default();
            }
        }
    }
    String::new()
}

fn pwsh_output(pwsh: &Path, expression: &str) -> Result<String, String> {
    let program = pwsh.display().to_string();
    let args = ["-NoLogo", "-NoProfile", "-Command", expression];
    match run_command(&program, &args) {
        Some((_, output)) => {
            let text = output.trim();
            if text.is_empty() {
                Err(format!("{program} printed nothing"))
            } else {
                Ok(text.to_owned())
            }
        }
        None => Err(format!("could not start {program}")),
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|error| format!("could not hash {}: {error}", path.display()))?;
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

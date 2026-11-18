//! PowerShell 7 MSI upgrade, ported from `Update-WinUtilPowerShellMSI`.
//!
//! When `Microsoft.PowerShell` is selected and a stable PowerShell 7 MSI is installed, the
//! original downloads the current MSI from the GitHub release and installs it in place instead of
//! letting WinGet handle it. Without an MSI install, a `pwsh.exe` already at the latest release
//! (MSIX, Store, or portable) is skipped; anything else falls through to the WinGet install.
//!
//! The decisions live here as pure functions. Everything that touches the machine goes through
//! [`MsiHost`]; the real implementation is in [`system`].

pub mod system;

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use super::run::{Manager, Outcome, Report};

/// The WinGet ID that triggers the MSI path.
pub const PACKAGE_ID: &str = "Microsoft.PowerShell";

const MULTIPLE_DETAIL: &str = "Multiple stable PowerShell MSI installations detected; upgrade each installation manually. No direct MSI upgrade was attempted.";
const PINS_DETAIL: &str = "WinGet pins exist or their state could not be verified; MSI upgrade skipped";
const CURRENT_DETAIL: &str = "PowerShell is already current";
const MSIX_DETAIL: &str = "PowerShell installed from the MSIX bundle; the existing MSI install was not changed";
const STANDALONE_CURRENT_DETAIL: &str = "PowerShell 7 is already current (not an MSI install); WinGet was not run";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub digest: Option<String>,
}

/// Everything the upgrade touches outside pure logic: registry, `pwsh.exe`, WinGet, GitHub, the
/// file system, and `msiexec`.
pub trait MsiHost {
    /// Install folders of stable PowerShell 7 MSI installs that contain `pwsh.exe`, deduplicated.
    fn msi_installs(&mut self) -> Vec<PathBuf>;
    /// A `pwsh.exe` on `PATH`, such as a Store or MSIX install. Asked only when no MSI install exists.
    fn standalone_pwsh(&mut self) -> Option<PathBuf>;
    /// `$PSVersionTable.PSVersion` as reported by `pwsh.exe`.
    fn installed_version(&mut self, pwsh: &Path) -> Result<String, String>;
    /// `ProcessArchitecture` as reported by `pwsh.exe`.
    fn architecture(&mut self, pwsh: &Path) -> Result<String, String>;
    /// Whether WinGet's pin list allows an upgrade (see [`pins_allow_upgrade`]).
    fn upgrade_allowed(&mut self) -> bool;
    fn latest_release(&mut self) -> Result<Release, String>;
    /// A fresh temp path ending in `.<extension>`.
    fn scratch_path(&mut self, extension: &str) -> PathBuf;
    fn download(&mut self, url: &str, dest: &Path) -> Result<(), String>;
    /// Checks the SHA-256 when GitHub supplies a digest, then the Microsoft Authenticode signature.
    fn verify(&mut self, msi: &Path, digest: Option<&str>) -> Result<(), String>;
    /// Runs `msiexec /i <msi> /qn /norestart` and returns its exit code.
    fn install(&mut self, msi: &Path) -> Result<i32, String>;
    /// Installs an MSIX bundle for the current user with `Add-AppxPackage`.
    fn install_msix(&mut self, bundle: &Path) -> Result<(), String>;
    fn remove(&mut self, path: &Path);
}

/// Runs the MSI path for `Microsoft.PowerShell`.
///
/// Returns `None` when no stable PowerShell 7 MSI is installed, so the caller runs WinGet.
pub fn update(host: &mut impl MsiHost) -> Option<Report> {
    let installs = host.msi_installs();
    let (outcome, exit_code, detail) = match installs.as_slice() {
        [] => return standalone(host),
        [_, _, ..] => (Outcome::Failed, -1, MULTIPLE_DETAIL.to_owned()),
        [install] => {
            if host.upgrade_allowed() {
                match upgrade(host, install.as_path()) {
                    Ok(done) => done,
                    Err(error) => (Outcome::Failed, -1, format!("PowerShell MSI upgrade failed: {error}")),
                }
            } else {
                (Outcome::Skipped, -1, PINS_DETAIL.to_owned())
            }
        }
    };
    Some(Report {
        package: PACKAGE_ID.to_owned(),
        manager: Manager::Msi,
        exit_code,
        outcome,
        detail,
        tail: Vec::new(),
    })
}

/// Without an MSI install, a `pwsh.exe` on `PATH` that is already at the latest release is skipped
/// so WinGet is not run. Returns `None` for an older install or when the versions cannot be
/// compared, so WinGet decides as the original does.
fn standalone(host: &mut impl MsiHost) -> Option<Report> {
    let pwsh = host.standalone_pwsh()?;
    if !standalone_is_current(host, &pwsh).ok()? {
        return None;
    }
    Some(Report {
        package: PACKAGE_ID.to_owned(),
        manager: Manager::Msi,
        exit_code: -1,
        outcome: Outcome::Skipped,
        detail: STANDALONE_CURRENT_DETAIL.to_owned(),
        tail: Vec::new(),
    })
}

/// Whether the `pwsh.exe` reports a version at least as new as the latest release.
fn standalone_is_current(host: &mut impl MsiHost, pwsh: &Path) -> Result<bool, String> {
    let installed = host.installed_version(pwsh)?;
    let release = host.latest_release()?;
    let latest = release.tag_name.trim().trim_start_matches('v');
    version_at_least(&installed, latest)
}

/// Upgrades one MSI install. Errors are reported by the caller as `Failed`.
fn upgrade(host: &mut impl MsiHost, install: &Path) -> Result<(Outcome, i32, String), String> {
    let pwsh = install.join("pwsh.exe");
    let installed = host.installed_version(&pwsh)?;
    let release = host.latest_release()?;
    let latest = release.tag_name.trim().trim_start_matches('v');
    if version_at_least(&installed, latest)? {
        return Ok((Outcome::Skipped, -1, CURRENT_DETAIL.to_owned()));
    }

    let arch = host.architecture(&pwsh)?.trim().to_ascii_lowercase();
    if !matches!(arch.as_str(), "x64" | "x86" | "arm64") {
        return Err(format!("unsupported PowerShell architecture: {arch}"));
    }
    let package = pick_package(&release.assets, &arch)
        .ok_or_else(|| format!("No PowerShell MSI or MSIX bundle found for {arch}"))?;
    let (asset, msix) = match package {
        Package::Msi(asset) => (asset, false),
        Package::Msix(asset) => (asset, true),
    };

    let scratch = host.scratch_path(if msix { "msixbundle" } else { "msi" });
    let result = host
        .download(&asset.browser_download_url, &scratch)
        .and_then(|()| host.verify(&scratch, asset.digest.as_deref()))
        .and_then(|()| {
            if msix {
                host.install_msix(&scratch).map(|()| 0)
            } else {
                host.install(&scratch)
            }
        });
    // Remove the installer whether or not the download, verification, or install succeeded.
    host.remove(&scratch);

    let code = result?;
    match code {
        _ if msix => Ok((Outcome::Succeeded, code, MSIX_DETAIL.to_owned())),
        0 | 3010 | 1641 => Ok((Outcome::Succeeded, code, success_detail(code).to_owned())),
        _ => Err(format!("PowerShell MSI installation failed with exit code {code}")),
    }
}

/// Matches the original's stable-release filter: `PowerShell 7[.n[.n[.n]]][-arch]` at the start of
/// the display name, excluding preview and daily builds.
static STABLE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^PowerShell\s+7(?:\.\d+){0,3}(?:-(?:x64|x86|arm64))?(?:\s|$)")
        .expect("static regex is valid")
});

pub fn is_stable_pwsh7(display_name: &str) -> bool {
    let lower = display_name.to_ascii_lowercase();
    STABLE_NAME.is_match(display_name) && !lower.contains("preview") && !lower.contains("daily")
}

/// WinGet pin list gate. Any pin table (a dashed separator line) or unverifiable state blocks the
/// MSI upgrade, leaving the decision to WinGet.
pub fn pins_allow_upgrade(output: &str, succeeded: bool) -> bool {
    if !succeeded || output.trim().is_empty() {
        return false;
    }
    !strip_ansi(output).lines().any(|line| {
        let line = line.trim();
        line.len() >= 3 && line.bytes().all(|b| b == b'-')
    })
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Drop the CSI sequence: ESC [ parameters... final letter.
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `[version]$installed -ge [version]$latest`, comparing numerically with missing parts as zero.
pub fn version_at_least(installed: &str, latest: &str) -> Result<bool, String> {
    let parse = |text: &str| -> Option<Vec<u32>> {
        text.trim().split('.').map(|part| part.parse::<u32>().ok()).collect()
    };
    let (Some(a), Some(b)) = (parse(installed), parse(latest)) else {
        return Err(format!("could not compare PowerShell versions '{installed}' and '{latest}'"));
    };
    let len = a.len().max(b.len());
    let padded = |v: &[u32]| (0..len).map(|i| v.get(i).copied().unwrap_or(0)).collect::<Vec<_>>();
    Ok(padded(&a) >= padded(&b))
}

/// First asset named `PowerShell-*-win-<arch>.msi`, matched case-insensitively like `-match`.
pub fn pick_asset<'a>(assets: &'a [Asset], arch: &str) -> Option<&'a Asset> {
    let prefix = "powershell-";
    let suffix = format!("-win-{arch}.msi");
    assets.iter().find(|asset| {
        let name = asset.name.to_ascii_lowercase();
        name.len() >= prefix.len() + suffix.len() && name.starts_with(prefix) && name.ends_with(&suffix)
    })
}

/// First asset named `PowerShell-*.msixbundle`. The bundle covers every architecture.
pub fn pick_msixbundle(assets: &[Asset]) -> Option<&Asset> {
    assets.iter().find(|asset| {
        let name = asset.name.to_ascii_lowercase();
        name.starts_with("powershell-") && name.ends_with(".msixbundle")
    })
}

/// Which installer a release offers; the MSI is preferred because it upgrades in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package<'a> {
    Msi(&'a Asset),
    Msix(&'a Asset),
}

/// The installer for a release: an MSI for `arch` when one exists, otherwise the MSIX bundle.
pub fn pick_package<'a>(assets: &'a [Asset], arch: &str) -> Option<Package<'a>> {
    pick_asset(assets, arch)
        .map(Package::Msi)
        .or_else(|| pick_msixbundle(assets).map(Package::Msix))
}

/// Compares a GitHub digest (`sha256:<hex>`, prefix optional) with the file's SHA-256 in hex.
pub fn digest_matches(expected: &str, actual_hex: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    let want = expected.strip_prefix("sha256:").unwrap_or(&expected);
    want == actual_hex.to_ascii_lowercase()
}

/// Authenticode gate: status must be `Valid` and the signer must be Microsoft Corporation.
pub fn signature_trusted(status: &str, subject: Option<&str>) -> bool {
    let Some(subject) = subject else { return false };
    // The signer's common name is the first RDN; the rest (O=, L=, ...) may vary.
    let common_name = subject.split(',').next().unwrap_or_default().trim();
    status.trim() == "Valid" && common_name.eq_ignore_ascii_case("CN=Microsoft Corporation")
}

fn success_detail(code: i32) -> &'static str {
    match code {
        3010 => "PowerShell MSI upgrade succeeded; a restart is required to finish",
        1641 => "PowerShell MSI upgrade succeeded; the installer started a restart",
        _ => "PowerShell MSI upgrade succeeded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_owned(),
            browser_download_url: format!("https://example.invalid/{name}"),
            digest: None,
        }
    }

    #[derive(Default)]
    struct Fake {
        installs: Vec<PathBuf>,
        standalone: Option<PathBuf>,
        version: Option<Result<String, String>>,
        arch: Option<Result<String, String>>,
        pins_allow: bool,
        release: Option<Result<Release, String>>,
        download: Option<Result<(), String>>,
        verify: Option<Result<(), String>>,
        install: Option<Result<i32, String>>,
        install_msix: Option<Result<(), String>>,
        calls: Vec<String>,
    }

    impl Fake {
        fn happy() -> Self {
            Self {
                installs: vec![PathBuf::from(r"C:\Program Files\PowerShell\7")],
                standalone: None,
                version: Some(Ok("7.5.2".to_owned())),
                arch: Some(Ok("X64".to_owned())),
                pins_allow: true,
                release: Some(Ok(Release {
                    tag_name: "v7.6.0".to_owned(),
                    assets: vec![asset("PowerShell-7.6.0-win-x64.msi")],
                })),
                download: Some(Ok(())),
                verify: Some(Ok(())),
                install: Some(Ok(0)),
                install_msix: Some(Ok(())),
                calls: Vec::new(),
            }
        }
    }

    impl MsiHost for Fake {
        fn msi_installs(&mut self) -> Vec<PathBuf> {
            self.installs.clone()
        }
        fn standalone_pwsh(&mut self) -> Option<PathBuf> {
            self.calls.push("standalone".into());
            self.standalone.clone()
        }
        fn installed_version(&mut self, _: &Path) -> Result<String, String> {
            self.calls.push("version".into());
            self.version.clone().expect("version set")
        }
        fn architecture(&mut self, _: &Path) -> Result<String, String> {
            self.calls.push("arch".into());
            self.arch.clone().expect("arch set")
        }
        fn upgrade_allowed(&mut self) -> bool {
            self.calls.push("pins".into());
            self.pins_allow
        }
        fn latest_release(&mut self) -> Result<Release, String> {
            self.calls.push("release".into());
            self.release.clone().expect("release set")
        }
        fn scratch_path(&mut self, extension: &str) -> PathBuf {
            PathBuf::from(format!("scratch.{extension}"))
        }
        fn download(&mut self, _: &str, _: &Path) -> Result<(), String> {
            self.calls.push("download".into());
            self.download.clone().expect("download set")
        }
        fn verify(&mut self, _: &Path, _: Option<&str>) -> Result<(), String> {
            self.calls.push("verify".into());
            self.verify.clone().expect("verify set")
        }
        fn install(&mut self, _: &Path) -> Result<i32, String> {
            self.calls.push("install".into());
            self.install.clone().expect("install set")
        }
        fn install_msix(&mut self, _: &Path) -> Result<(), String> {
            self.calls.push("install_msix".into());
            self.install_msix.clone().expect("install_msix set")
        }
        fn remove(&mut self, _: &Path) {
            self.calls.push("remove".into());
        }
    }

    #[test]
    fn no_msi_install_returns_none_so_winget_runs() {
        let mut host = Fake { installs: Vec::new(), ..Fake::happy() };
        assert!(update(&mut host).is_none());
        assert_eq!(host.calls, ["standalone"]);
    }

    #[test]
    fn current_standalone_pwsh_is_skipped_without_winget_or_download() {
        let mut host = Fake {
            installs: Vec::new(),
            standalone: Some(PathBuf::from("pwsh.exe")),
            version: Some(Ok("7.6.0".to_owned())),
            ..Fake::happy()
        };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Skipped);
        assert_eq!(report.manager, Manager::Msi);
        assert_eq!(report.detail, STANDALONE_CURRENT_DETAIL);
        assert_eq!(host.calls, ["standalone", "version", "release"]);
    }

    #[test]
    fn older_standalone_pwsh_falls_through_to_winget() {
        let mut host = Fake {
            installs: Vec::new(),
            standalone: Some(PathBuf::from("pwsh.exe")),
            version: Some(Ok("7.5.2".to_owned())),
            ..Fake::happy()
        };
        assert!(update(&mut host).is_none());
        assert_eq!(host.calls, ["standalone", "version", "release"]);
    }

    #[test]
    fn standalone_version_that_cannot_be_compared_falls_through_to_winget() {
        let mut host = Fake {
            installs: Vec::new(),
            standalone: Some(PathBuf::from("pwsh.exe")),
            version: Some(Ok("7.8.0-preview.1".to_owned())),
            ..Fake::happy()
        };
        assert!(update(&mut host).is_none());
        assert!(!host.calls.contains(&"download".to_owned()));
    }

    #[test]
    fn standalone_release_lookup_failure_falls_through_without_installing() {
        let mut host = Fake {
            installs: Vec::new(),
            standalone: Some(PathBuf::from("pwsh.exe")),
            release: Some(Err("GitHub release lookup failed: offline".to_owned())),
            ..Fake::happy()
        };
        assert!(update(&mut host).is_none());
        assert!(!host.calls.contains(&"install".to_owned()));
        assert!(!host.calls.contains(&"install_msix".to_owned()));
    }

    #[test]
    fn msi_install_takes_precedence_over_standalone_pwsh() {
        let mut host = Fake { standalone: Some(PathBuf::from("pwsh.exe")), ..Fake::happy() };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Succeeded);
        assert!(!host.calls.contains(&"standalone".to_owned()));
    }

    #[test]
    fn multiple_installs_fail_without_touching_anything() {
        let mut host = Fake {
            installs: vec![PathBuf::from("a"), PathBuf::from("b")],
            ..Fake::happy()
        };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.exit_code, -1);
        assert_eq!(report.manager, Manager::Msi);
        assert!(report.detail.starts_with("Multiple stable PowerShell MSI installations"));
        assert!(host.calls.is_empty());
    }

    #[test]
    fn pin_gate_skips_before_any_release_lookup_or_download() {
        let mut host = Fake { pins_allow: false, ..Fake::happy() };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Skipped);
        assert_eq!(report.detail, PINS_DETAIL);
        assert_eq!(host.calls, ["pins"]);
    }

    #[test]
    fn current_version_is_skipped_without_download() {
        let mut host = Fake {
            version: Some(Ok("7.6.0".to_owned())),
            ..Fake::happy()
        };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Skipped);
        assert_eq!(report.detail, CURRENT_DETAIL);
        assert!(!host.calls.contains(&"download".to_owned()));
    }

    #[test]
    fn upgrade_downloads_verifies_installs_then_removes() {
        let mut host = Fake::happy();
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Succeeded);
        assert_eq!(report.exit_code, 0);
        assert_eq!(report.detail, "PowerShell MSI upgrade succeeded");
        assert_eq!(
            host.calls,
            ["pins", "version", "release", "arch", "download", "verify", "install", "remove"]
        );
    }

    #[test]
    fn reboot_codes_succeed_with_restart_text() {
        let mut host = Fake { install: Some(Ok(3010)), ..Fake::happy() };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Succeeded);
        assert_eq!(report.exit_code, 3010);
        assert!(report.detail.contains("restart is required"), "{}", report.detail);

        let mut host = Fake { install: Some(Ok(1641)), ..Fake::happy() };
        let report = update(&mut host).unwrap();
        assert!(report.detail.contains("the installer started a restart"), "{}", report.detail);
    }

    #[test]
    fn msiexec_failure_fails_and_still_removes_the_file() {
        let mut host = Fake { install: Some(Ok(1603)), ..Fake::happy() };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.exit_code, -1);
        assert_eq!(
            report.detail,
            "PowerShell MSI upgrade failed: PowerShell MSI installation failed with exit code 1603"
        );
        assert_eq!(host.calls.last().map(String::as_str), Some("remove"));
    }

    #[test]
    fn failed_verification_never_runs_msiexec() {
        let mut host = Fake {
            verify: Some(Err("PowerShell MSI signature verification failed".to_owned())),
            ..Fake::happy()
        };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Failed);
        assert!(report.detail.ends_with("signature verification failed"), "{}", report.detail);
        assert!(!host.calls.contains(&"install".to_owned()));
        assert_eq!(host.calls.last().map(String::as_str), Some("remove"));
    }

    #[test]
    fn failed_download_removes_partial_file() {
        let mut host = Fake { download: Some(Err("connection reset".to_owned())), ..Fake::happy() };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Failed);
        assert!(report.detail.contains("connection reset"), "{}", report.detail);
        assert_eq!(host.calls.last().map(String::as_str), Some("remove"));
    }

    #[test]
    fn missing_architecture_asset_fails_before_download() {
        let mut host = Fake {
            release: Some(Ok(Release {
                tag_name: "v7.6.0".to_owned(),
                assets: vec![asset("PowerShell-7.6.0-win-arm64.msi")],
            })),
            ..Fake::happy()
        };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.detail, "PowerShell MSI upgrade failed: No PowerShell MSI or MSIX bundle found for x64");
        assert!(!host.calls.contains(&"download".to_owned()));
    }

    #[test]
    fn msixbundle_is_installed_when_no_msi_asset_exists() {
        let mut host = Fake {
            release: Some(Ok(Release {
                tag_name: "v7.7.0".to_owned(),
                assets: vec![asset("PowerShell-7.7.0.msixbundle")],
            })),
            ..Fake::happy()
        };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Succeeded);
        assert_eq!(report.exit_code, 0);
        assert!(host.calls.contains(&"install_msix".to_owned()));
        assert!(!host.calls.contains(&"install".to_owned()));
        assert_eq!(host.calls.last().map(String::as_str), Some("remove"));
    }

    #[test]
    fn msixbundle_install_failure_is_reported_and_bundle_removed() {
        let mut host = Fake {
            release: Some(Ok(Release {
                tag_name: "v7.7.0".to_owned(),
                assets: vec![asset("PowerShell-7.7.0.msixbundle")],
            })),
            install_msix: Some(Err("MSIX bundle installation failed with exit code 1".to_owned())),
            ..Fake::happy()
        };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(
            report.detail,
            "PowerShell MSI upgrade failed: MSIX bundle installation failed with exit code 1"
        );
        assert_eq!(host.calls.last().map(String::as_str), Some("remove"));
    }

    #[test]
    fn msi_is_preferred_over_msixbundle() {
        let assets = [asset("PowerShell-7.6.0.msixbundle"), asset("PowerShell-7.6.0-win-x64.msi")];
        assert_eq!(pick_package(&assets, "x64"), Some(Package::Msi(&assets[1])));
    }

    #[test]
    fn msixbundle_is_picked_when_no_msi_matches_arch() {
        let assets = [asset("PowerShell-7.7.0-win-x64.zip"), asset("PowerShell-7.7.0.msixbundle")];
        assert_eq!(pick_package(&assets, "arm64"), Some(Package::Msix(&assets[1])));
    }

    #[test]
    fn zip_alone_is_not_an_installer() {
        let assets = [asset("PowerShell-7.7.0-win-x64.zip")];
        assert_eq!(pick_package(&assets, "x64"), None);
    }

    #[test]
    fn unsupported_architecture_fails() {
        let mut host = Fake { arch: Some(Ok("IA64".to_owned())), ..Fake::happy() };
        let report = update(&mut host).unwrap();
        assert_eq!(report.outcome, Outcome::Failed);
        assert!(report.detail.contains("unsupported PowerShell architecture: ia64"), "{}", report.detail);
    }

    #[test]
    fn stable_name_filter_matches_original_regex() {
        for name in ["PowerShell 7", "PowerShell 7.4.6-x64", "PowerShell 7.5.2 (x64)", "powershell 7.4.0-arm64"] {
            assert!(is_stable_pwsh7(name), "{name}");
        }
        for name in [
            "PowerShell 7-preview",
            "PowerShell 7.6.0-preview.4",
            "PowerShell 7 Daily",
            "PowerShell 6.2",
            "PowerShell 70",
            "Windows PowerShell",
        ] {
            assert!(!is_stable_pwsh7(name), "{name}");
        }
    }

    #[test]
    fn pin_table_or_unknown_state_blocks_upgrade() {
        assert!(!pins_allow_upgrade("", true));
        assert!(!pins_allow_upgrade("No pinned packages", false));
        assert!(!pins_allow_upgrade("Name Id Version\n----------\nGit Git.Git 2.0", true));
        assert!(!pins_allow_upgrade("\u{1b}[31m---\u{1b}[0m", true));
        assert!(pins_allow_upgrade("There are no pinned packages.", true));
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(version_at_least("7.6.0", "7.6.0"), Ok(true));
        assert_eq!(version_at_least("7.5.9", "7.6.0"), Ok(false));
        assert_eq!(version_at_least("7.10.0", "7.9.0"), Ok(true));
        assert_eq!(version_at_least("7.6", "7.6.0"), Ok(true));
        assert!(version_at_least("not-a-version", "7.6.0").is_err());
    }

    #[test]
    fn asset_selection_is_case_insensitive_and_arch_exact() {
        let assets = [
            asset("PowerShell-7.6.0-win-x64.zip"),
            asset("PowerShell-7.6.0-win-arm64.msi"),
            asset("POWERSHELL-7.6.0-WIN-X64.MSI"),
            asset("PowerShell-7.6.0-win-x64.msi"),
        ];
        assert_eq!(pick_asset(&assets, "x64").unwrap().name, "POWERSHELL-7.6.0-WIN-X64.MSI");
        assert_eq!(pick_asset(&assets, "arm64").unwrap().name, "PowerShell-7.6.0-win-arm64.msi");
        assert!(pick_asset(&assets, "x86").is_none());
        assert!(pick_asset(&[asset("PowerShell-win-x64.msi")], "x64").is_none());
    }

    #[test]
    fn digest_accepts_optional_prefix_and_any_hex_case() {
        assert!(digest_matches("sha256:ABCDEF", "abcdef"));
        assert!(digest_matches("abcdef", "ABCDEF"));
        assert!(!digest_matches("sha256:abcdef", "abcdee"));
    }

    #[test]
    fn signature_requires_valid_status_and_exact_microsoft_signer() {
        let microsoft = "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US";
        assert!(signature_trusted("Valid", Some(microsoft)));
        assert!(signature_trusted("Valid", Some("CN=Microsoft Corporation")));
        assert!(!signature_trusted("HashMismatch", Some(microsoft)));
        assert!(!signature_trusted("Valid", Some("CN=Microsoft Corporation Fake")));
        assert!(!signature_trusted("Valid", Some("CN=Evil, O=Microsoft Corporation")));
        assert!(!signature_trusted("Valid", None));
    }
}

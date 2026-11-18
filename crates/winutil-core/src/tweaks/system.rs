//! Real [`TweakHost`]: the registry through `winreg`, services through `sc.exe`, scripts through
//! `powershell.exe`, and downloads through [`crate::net`].
//!
//! Only the glue lives here. The decisions it feeds are tested in the parent module and in
//! `tests/tweaks_run.rs`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use winreg::RegKey;
use winreg::RegValue as RawValue;
use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ, KEY_WRITE, RegType as RawType,
};

use super::run::{TweakHost, Written};
use super::{Hive, RegWrite, split_path};
use crate::config::registry::{RegType, RegValue};
use crate::config::tweaks::StartupType;
use crate::install::run::{last_line, run_command};
use crate::net;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const SERVICES_KEY: &str = r"SYSTEM\CurrentControlSet\Services";

/// The production host. Stateless: each call talks to the machine directly.
pub struct SystemTweaks;

impl TweakHost for SystemTweaks {
    fn read_registry(&mut self, path: &str, name: &str) -> Result<Option<String>, String> {
        let (hive, sub) = split_path(path).ok_or_else(|| format!("unknown registry hive in {path}"))?;
        let Some(key) = open_read(hive, sub).map_err(|e| format!("could not open {path}: {e}"))? else {
            return Ok(None);
        };
        match key.get_raw_value(name) {
            Ok(raw) => decode(&raw).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("could not read {name} under {path}: {error}")),
        }
    }

    fn write_registry(&mut self, write: &RegWrite) -> Result<Written, String> {
        let (hive, sub) =
            split_path(&write.path).ok_or_else(|| format!("unknown registry hive in {}", write.path))?;
        match &write.value {
            RegValue::Remove => {
                let key = match predef(hive).open_subkey_with_flags(sub, KEY_WRITE) {
                    Ok(key) => key,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        return Ok(Written::AlreadyAbsent);
                    }
                    Err(error) => return Err(format!("could not open {}: {error}", write.path)),
                };
                match key.delete_value(&write.name) {
                    Ok(()) => Ok(Written::Removed),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Written::AlreadyAbsent),
                    Err(error) => Err(error.to_string()),
                }
            }
            RegValue::Set(text) => {
                let (key, _) = predef(hive)
                    .create_subkey(sub)
                    .map_err(|e| format!("could not create {}: {e}", write.path))?;
                match write.kind {
                    RegType::DWord => {
                        let number: u32 = text
                            .parse()
                            .map_err(|_| format!("'{text}' is not a DWORD value"))?;
                        key.set_value(&write.name, &number)
                    }
                    RegType::QWord => {
                        let number: u64 = text
                            .parse()
                            .map_err(|_| format!("'{text}' is not a QWORD value"))?;
                        key.set_value(&write.name, &number)
                    }
                    RegType::String => key.set_value(&write.name, text),
                }
                .map_err(|e| e.to_string())?;
                Ok(Written::Set)
            }
        }
    }

    fn read_service(&mut self, name: &str) -> Result<Option<StartupType>, String> {
        let path = format!(r"{SERVICES_KEY}\{name}");
        let Some(key) = open_read(Hive::LocalMachine, &path)
            .map_err(|e| format!("could not open service {name}: {e}"))?
        else {
            return Ok(None);
        };
        let start: u32 = key
            .get_value("Start")
            .map_err(|e| format!("could not read the startup type of {name}: {e}"))?;
        startup_from_start(start).map(Some)
    }

    fn set_service(&mut self, name: &str, target: StartupType) -> Result<(), String> {
        let mode = match target {
            StartupType::Automatic => "auto",
            StartupType::Manual => "demand",
            StartupType::Disabled => "disabled",
        };
        match run_command("sc.exe", &["config", name, "start=", mode]) {
            Some((0, _)) => Ok(()),
            Some((code, output)) => Err(format!("sc.exe exit code {code}: {}", last_line(&output))),
            None => Err("sc.exe could not be started".to_owned()),
        }
    }

    fn run_script(&mut self, script: &str) -> Result<(), String> {
        crate::powershell::run_script(script)
    }

    fn run_program(&mut self, program: &str, args: &[&str]) -> Option<(i32, String)> {
        run_command(program, args)
    }

    fn download(&mut self, url: &str, dest: &Path) -> Result<(), String> {
        net::download(url, dest, DOWNLOAD_TIMEOUT)
    }

    fn launch(&mut self, program: &Path) -> Result<(), String> {
        Command::new(program)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not start {}: {e}", program.display()))
    }

    fn winutil_dir(&self) -> Result<PathBuf, String> {
        let local = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| "LOCALAPPDATA is not set".to_owned())?;
        Ok(PathBuf::from(local).join("winutil"))
    }
}

fn predef(hive: Hive) -> RegKey {
    RegKey::predef(match hive {
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::Users => HKEY_USERS,
    })
}

/// Opens `sub` read-only. `Ok(None)` when the key does not exist.
fn open_read(hive: Hive, sub: &str) -> io::Result<Option<RegKey>> {
    match predef(hive).open_subkey_with_flags(sub, KEY_READ) {
        Ok(key) => Ok(Some(key)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Maps a raw registry value to the text the tweak logic compares.
fn decode(raw: &RawValue) -> Result<String, String> {
    match &raw.vtype {
        RawType::REG_DWORD => <[u8; 4]>::try_from(raw.bytes.as_slice())
            .map(|bytes| u32::from_le_bytes(bytes).to_string())
            .map_err(|_| "malformed DWORD value".to_owned()),
        RawType::REG_QWORD => <[u8; 8]>::try_from(raw.bytes.as_slice())
            .map(|bytes| u64::from_le_bytes(bytes).to_string())
            .map_err(|_| "malformed QWORD value".to_owned()),
        RawType::REG_SZ | RawType::REG_EXPAND_SZ => {
            let units: Vec<u16> = raw
                .bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
        }
        other => Err(format!("unsupported registry value type {other:?}")),
    }
}

/// Maps the `Start` value of a service key to a startup type. Other values (boot, system) have no
/// tweak equivalent and are reported as errors.
pub fn startup_from_start(start: u32) -> Result<StartupType, String> {
    match start {
        2 => Ok(StartupType::Automatic),
        3 => Ok(StartupType::Manual),
        4 => Ok(StartupType::Disabled),
        other => Err(format!("startup value {other} has no tweak equivalent")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winreg::enums::{REG_DWORD, REG_EXPAND_SZ, REG_QWORD, REG_SZ};

    fn raw(vtype: RawType, bytes: Vec<u8>) -> RawValue {
        RawValue { vtype, bytes }
    }

    #[test]
    fn decodes_dword_and_qword_as_decimal() {
        assert_eq!(decode(&raw(REG_DWORD, 258u32.to_le_bytes().to_vec())).unwrap(), "258");
        assert_eq!(
            decode(&raw(REG_QWORD, u64::MAX.to_le_bytes().to_vec())).unwrap(),
            u64::MAX.to_string()
        );
    }

    #[test]
    fn decodes_utf16_strings_and_drops_the_terminator() {
        let bytes: Vec<u8> = "Hi\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&raw(REG_SZ, bytes.clone())).unwrap(), "Hi");
        assert_eq!(decode(&raw(REG_EXPAND_SZ, bytes)).unwrap(), "Hi");
    }

    #[test]
    fn rejects_malformed_numbers_and_other_types() {
        assert!(decode(&raw(REG_DWORD, vec![1, 2])).is_err());
        assert!(decode(&raw(REG_QWORD, vec![1, 2, 3])).is_err());
        assert!(decode(&raw(RawType::REG_BINARY, vec![1])).is_err());
    }

    #[test]
    fn start_values_map_to_startup_types() {
        assert_eq!(startup_from_start(2), Ok(StartupType::Automatic));
        assert_eq!(startup_from_start(3), Ok(StartupType::Manual));
        assert_eq!(startup_from_start(4), Ok(StartupType::Disabled));
        assert!(startup_from_start(1).is_err(), "system drivers have no tweak equivalent");
    }
}

//! HTTP helpers shared by the features that download files.

use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::Duration;

/// An HTTP agent that gives up after `timeout` for the whole request.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

/// Downloads `url` into `dest`, creating the parent directory when needed. A partial file is left
/// behind on failure; callers that need atomicity must write elsewhere first.
pub fn download(url: &str, dest: &Path, timeout: Duration) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut response = agent(timeout)
        .get(url)
        .header("User-Agent", "winutil-rs")
        .call()
        .map_err(|error| format!("download failed: {error}"))?;
    let mut file = File::create(dest)
        .map_err(|error| format!("could not create {}: {error}", dest.display()))?;
    let mut body = response.body_mut().as_reader();
    io::copy(&mut body, &mut file).map_err(|error| format!("download interrupted: {error}"))?;
    Ok(())
}

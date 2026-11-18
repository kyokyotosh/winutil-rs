//! Typed model of the declarative data in `config/*.json`.
//!
//! The JSON files are the data the PowerShell original reads, so they remain the source of truth.
//! They are embedded at compile time; nothing is read from disk at startup.

mod de;

pub mod applications;
pub mod appx;
pub mod dns;
pub mod feature;
pub mod navigation;
pub mod registry;
pub mod tweaks;

use std::error::Error;
use std::fmt;

use indexmap::IndexMap;
use serde::de::DeserializeOwned;

pub use applications::Application;
pub use appx::AppxPackage;
pub use dns::DnsProvider;
pub use feature::Feature;
pub use navigation::NavItem;
pub use tweaks::Tweak;

/// UI theme: WPF resource name → value.
pub type Theme = IndexMap<String, String>;

/// Preset name → tweak and appx keys it applies, in file order.
pub type Presets = IndexMap<String, Vec<String>>;

const APPLICATIONS: &str = include_str!("../../../../config/applications.json");
const APP_NAVIGATION: &str = include_str!("../../../../config/appnavigation.json");
const APPX: &str = include_str!("../../../../config/appx.json");
const DNS: &str = include_str!("../../../../config/dns.json");
const FEATURES: &str = include_str!("../../../../config/feature.json");
const PRESETS: &str = include_str!("../../../../config/preset.json");
const THEMES: &str = include_str!("../../../../config/themes.json");
const TWEAKS: &str = include_str!("../../../../config/tweaks.json");

/// Every `config/*.json` file, parsed.
///
/// Maps keep the file's entry order, which is the order the UI shows them in.
#[derive(Debug, Clone)]
pub struct Config {
    pub applications: IndexMap<String, Application>,
    pub app_navigation: IndexMap<String, NavItem>,
    pub appx: IndexMap<String, AppxPackage>,
    pub dns: IndexMap<String, DnsProvider>,
    pub features: IndexMap<String, Feature>,
    pub presets: Presets,
    pub themes: IndexMap<String, Theme>,
    pub tweaks: IndexMap<String, Tweak>,
}

/// A config file that failed to parse.
#[derive(Debug)]
pub struct ConfigError {
    file: &'static str,
    source: serde_json_lenient::Error,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config/{} is invalid: {}", self.file, self.source)
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Parses the embedded config.
pub fn load() -> Result<Config, ConfigError> {
    Ok(Config {
        applications: parse("applications.json", APPLICATIONS)?,
        app_navigation: parse("appnavigation.json", APP_NAVIGATION)?,
        appx: parse("appx.json", APPX)?,
        dns: parse("dns.json", DNS)?,
        features: parse("feature.json", FEATURES)?,
        presets: parse("preset.json", PRESETS)?,
        themes: parse("themes.json", THEMES)?,
        tweaks: parse("tweaks.json", TWEAKS)?,
    })
}

fn parse<T: DeserializeOwned>(file: &'static str, text: &str) -> Result<T, ConfigError> {
    // tweaks.json and feature.json hold multi-line PowerShell inside string literals, so raw
    // CR/LF must be accepted. serde_json_lenient also accepts trailing commas and comments by
    // default; both are turned off so the data stays strict JSON apart from that.
    let mut de = serde_json_lenient::Deserializer::from_slice_with_options(
        text.as_bytes(),
        false,
        true,
        false,
        false,
        false,
    );
    de.set_ignore_trailing_commas(false);
    de.set_allow_comments(false);
    let value = T::deserialize(&mut de).map_err(|source| ConfigError { file, source })?;
    de.end().map_err(|source| ConfigError { file, source })?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_raw_newlines_inside_strings() {
        let parsed: Vec<String> = parse("t.json", "[\"a\r\nb\"]").unwrap();
        assert_eq!(parsed, ["a\r\nb"]);
    }

    #[test]
    fn rejects_tab_inside_string() {
        assert!(parse::<Vec<String>>("t.json", "[\"a\tb\"]").is_err());
    }

    #[test]
    fn rejects_trailing_comma() {
        assert!(parse::<Vec<String>>("t.json", "[\"a\",]").is_err());
    }

    #[test]
    fn rejects_comment() {
        assert!(parse::<Vec<String>>("t.json", "[\"a\" /* note */]").is_err());
    }
}

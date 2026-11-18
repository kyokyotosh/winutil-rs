//! `applications.json`: apps the Install tab can install.

use serde::Deserialize;

/// One app. Keys are unprefixed here; the original adds `WPFInstall` when it compiles the data.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Application {
    /// Maintainer note. Present on one entry only.
    #[serde(rename = "_comment", default)]
    pub comment: Option<String>,
    pub category: String,
    pub content: String,
    pub description: String,
    pub link: String,
    pub winget: String,
    pub choco: Option<String>,
    pub foss: Option<bool>,
}

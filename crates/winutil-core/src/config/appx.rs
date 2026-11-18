//! `appx.json`: Windows Store packages the Appx removal tab can remove.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppxPackage {
    pub category: String,
    pub content: String,
    pub description: String,
    #[serde(deserialize_with = "super::de::quoted")]
    pub panel: u8,
    pub package_id: String,
    pub store_id: Option<String>,
}

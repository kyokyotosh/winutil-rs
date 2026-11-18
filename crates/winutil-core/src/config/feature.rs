//! `feature.json`: Windows optional features and one-shot fixes on the Features and Fixes tabs.

use serde::Deserialize;

use super::tweaks::WidgetKind;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Feature {
    pub category: String,
    #[serde(rename = "Content")]
    pub content: String,
    #[serde(rename = "Description", default)]
    pub description: Option<String>,
    pub link: String,
    #[serde(deserialize_with = "super::de::quoted")]
    pub panel: u8,
    #[serde(rename = "Type", default)]
    pub kind: Option<WidgetKind>,
    #[serde(rename = "ButtonWidth", default, deserialize_with = "super::de::opt_quoted")]
    pub button_width: Option<u32>,
    /// DISM optional-feature names this entry enables or disables.
    #[serde(rename = "feature", default)]
    pub optional_features: Vec<String>,
    #[serde(rename = "InvokeScript", default)]
    pub invoke_script: Vec<String>,
    #[serde(default)]
    pub function: Option<String>,
}

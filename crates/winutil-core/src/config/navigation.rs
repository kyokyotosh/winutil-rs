//! `appnavigation.json`: the Install tab's package-manager choice and action buttons.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum NavKind {
    Button,
    Note,
    RadioButton,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NavItem {
    pub category: String,
    pub content: String,
    pub description: String,
    #[serde(rename = "Type")]
    pub kind: NavKind,
    #[serde(deserialize_with = "super::de::quoted")]
    pub order: u32,
    #[serde(default)]
    pub checked: bool,
    pub group_name: Option<String>,
}

//! `tweaks.json`: Windows tweaks (registry, services, scripts) on the Tweaks tab.

use indexmap::IndexMap;
use serde::Deserialize;

use super::de;
use super::registry::RegistryChange;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum WidgetKind {
    Button,
    Combobox,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum StartupType {
    Automatic,
    Manual,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServiceChange {
    pub name: String,
    pub startup_type: StartupType,
    pub original_type: StartupType,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Tweak {
    pub category: String,
    #[serde(rename = "Content")]
    pub content: String,
    #[serde(rename = "Description", default)]
    pub description: Option<String>,
    pub link: String,
    #[serde(deserialize_with = "de::quoted")]
    pub panel: u8,
    #[serde(rename = "Type", default)]
    pub kind: Option<WidgetKind>,
    #[serde(rename = "ButtonWidth", default, deserialize_with = "de::opt_quoted")]
    pub button_width: Option<u32>,
    #[serde(rename = "Checked", default, deserialize_with = "de::quoted_bool")]
    pub checked: bool,
    #[serde(default)]
    pub registry: Vec<RegistryChange>,
    #[serde(default)]
    pub service: Vec<ServiceChange>,
    #[serde(rename = "InvokeScript", default)]
    pub invoke_script: Vec<String>,
    #[serde(rename = "UndoScript", default)]
    pub undo_script: Vec<String>,
    #[serde(rename = "ComboItems", default)]
    pub combo_items: Option<String>,
    #[serde(rename = "ComboDescriptions", default)]
    pub combo_descriptions: IndexMap<String, String>,
}

impl Tweak {
    /// Choices of a combobox tweak, in display order. `ComboItems` is `|`-separated in the JSON.
    pub fn combo_choices(&self) -> Vec<&str> {
        self.combo_items
            .as_deref()
            .map(|items| items.split('|').collect())
            .unwrap_or_default()
    }
}

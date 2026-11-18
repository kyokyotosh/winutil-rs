//! Registry values a tweak writes.

use indexmap::IndexMap;
use serde::Deserialize;

use super::de;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum RegType {
    DWord,
    QWord,
    String,
}

/// A registry value. The JSON writes `"<RemoveEntry>"` to mean "delete the value".
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum RegValue {
    Set(String),
    Remove,
}

const REMOVE_ENTRY: &str = "<RemoveEntry>";

impl From<String> for RegValue {
    fn from(text: String) -> Self {
        if text == REMOVE_ENTRY {
            Self::Remove
        } else {
            Self::Set(text)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RegistryChange {
    pub path: String,
    pub name: String,
    #[serde(rename = "Type")]
    pub kind: RegType,
    #[serde(default)]
    pub value: Option<RegValue>,
    #[serde(default)]
    pub original_value: Option<RegValue>,
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default, deserialize_with = "de::opt_quoted_bool")]
    pub default_state: Option<bool>,
    /// Per-choice values for combobox tweaks, keyed by choice name.
    #[serde(default)]
    pub values: IndexMap<String, RegValue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_entry_sentinel_becomes_remove() {
        let change: RegistryChange = serde_json_lenient::from_str(
            r#"{"Path":"HKLM:\\X","Name":"A","Type":"DWord","Value":"<RemoveEntry>","OriginalValue":"0"}"#,
        )
        .unwrap();
        assert_eq!(change.value, Some(RegValue::Remove));
        assert_eq!(change.original_value, Some(RegValue::Set("0".into())));
        assert_eq!(change.kind, RegType::DWord);
    }

    #[test]
    fn combobox_values_are_keyed_by_choice_and_missing_value_is_none() {
        let change: RegistryChange = serde_json_lenient::from_str(
            r#"{"Path":"HKLM:\\X","Name":"A","Type":"DWord","DefaultValue":"0","DefaultState":"false",
                "Values":{"Enabled":"<RemoveEntry>","Fully Disabled":"1"}}"#,
        )
        .unwrap();
        assert_eq!(change.value, None);
        assert_eq!(change.default_state, Some(false));
        assert_eq!(change.values.get("Enabled"), Some(&RegValue::Remove));
        assert_eq!(change.values.get("Fully Disabled"), Some(&RegValue::Set("1".into())));
    }
}

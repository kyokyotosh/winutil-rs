//! Deserializers for values the JSON stores as strings (`"1"`, `"True"`).

use std::fmt::Display;
use std::str::FromStr;

use serde::Deserialize;
use serde::de::{self, Deserializer, Unexpected};

/// Parses a numeric string such as `"1"`.
pub fn quoted<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    let text = String::deserialize(deserializer)?;
    text.parse().map_err(de::Error::custom)
}

/// Optional form of [`quoted`]. Pair with `#[serde(default)]`.
pub fn opt_quoted<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    Option::<String>::deserialize(deserializer)?
        .map(|text| text.parse().map_err(de::Error::custom))
        .transpose()
}

/// Parses `"True"` or `"False"`, in any casing.
pub fn quoted_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let text = String::deserialize(deserializer)?;
    parse_bool(&text).ok_or_else(|| invalid_bool(&text))
}

/// Optional form of [`quoted_bool`]. Pair with `#[serde(default)]`.
pub fn opt_quoted_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)?
        .map(|text| parse_bool(&text).ok_or_else(|| invalid_bool(&text)))
        .transpose()
}

fn parse_bool(text: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn invalid_bool<E: de::Error>(text: &str) -> E {
    E::invalid_value(Unexpected::Str(text), &"\"true\" or \"false\"")
}

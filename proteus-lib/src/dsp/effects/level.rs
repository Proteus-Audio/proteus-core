//! Helpers for parsing and converting linear and dB gain values.

use serde::de::{Error as DeError, Visitor};
use serde::Deserializer;
use std::fmt;

/// Convert a dB value to linear gain.
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Convert a linear gain to dB.
pub fn linear_to_db(value: f32) -> f32 {
    let v = value.max(f32::MIN_POSITIVE);
    20.0 * v.log10()
}

/// Deserialize a linear gain value that may be expressed in dB.
pub fn deserialize_linear_gain<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_gain(deserializer, parse_linear_or_db_str_to_linear)
}

/// Deserialize a dB gain value that may be expressed as linear.
pub fn deserialize_db_gain<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_gain(deserializer, parse_linear_or_db_str_to_db)
}

fn deserialize_gain<'de, D>(
    deserializer: D,
    parse_str: fn(&str) -> Option<f32>,
) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    struct GainVisitor {
        parse_str: fn(&str) -> Option<f32>,
    }

    impl<'de> Visitor<'de> for GainVisitor {
        type Value = f32;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a number or a string like \"6db\"")
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            Ok(value as f32)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            Ok(value as f32)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            Ok(value as f32)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            (self.parse_str)(value).ok_or_else(|| {
                DeError::custom(format!("invalid gain value \"{}\"", value))
            })
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: DeError,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(GainVisitor { parse_str })
}

fn parse_linear_or_db_str_to_linear(value: &str) -> Option<f32> {
    if let Some(db) = parse_db_suffix(value) {
        return Some(db_to_linear(db));
    }
    parse_number(value)
}

fn parse_linear_or_db_str_to_db(value: &str) -> Option<f32> {
    if let Some(db) = parse_db_suffix(value) {
        return Some(db);
    }
    parse_number(value).map(linear_to_db)
}

fn parse_db_suffix(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let db_part = lower.strip_suffix("db")?;
    db_part.trim().parse::<f32>().ok()
}

fn parse_number(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f32>().ok()
}

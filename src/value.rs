//! Conversions between `SeaORM` [`Value`]s, JSON and record keys.
//!
//! Records are read with `ModelTrait::get` and written with
//! `ActiveModelTrait::try_set`, so the admin works with any entity regardless
//! of its serde derives. Incoming JSON is turned into a list of candidate
//! [`Value`]s derived from the column type; the first candidate the active
//! model accepts wins.

use std::str::FromStr;

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, Value,
    prelude::{
        ChronoDate, ChronoDateTime, ChronoDateTimeLocal, ChronoDateTimeUtc,
        ChronoDateTimeWithTimeZone, ChronoTime, Decimal, Uuid,
    },
    sea_query::{ColumnType, Enum, OptionEnum},
};
use serde_json::Value as Json;

/// Largest integer JavaScript represents exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// A JSON number, or a string for integers JavaScript would round; the API
/// accepts integers as strings, so the UI can send them back unchanged.
fn int_json(value: i64) -> Json {
    if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
        Json::from(value)
    } else {
        Json::String(value.to_string())
    }
}

fn uint_json(value: u64) -> Json {
    if value <= MAX_SAFE_INTEGER.unsigned_abs() {
        Json::from(value)
    } else {
        Json::String(value.to_string())
    }
}

/// Convert a database value into the JSON sent to the UI.
pub(crate) fn to_json(value: &Value) -> Json {
    match value {
        Value::Bool(v) => v.map_or(Json::Null, Json::Bool),
        Value::TinyInt(v) => v.map_or(Json::Null, Json::from),
        Value::SmallInt(v) => v.map_or(Json::Null, Json::from),
        Value::Int(v) => v.map_or(Json::Null, Json::from),
        Value::BigInt(v) => v.map_or(Json::Null, int_json),
        Value::TinyUnsigned(v) => v.map_or(Json::Null, Json::from),
        Value::SmallUnsigned(v) => v.map_or(Json::Null, Json::from),
        Value::Unsigned(v) => v.map_or(Json::Null, Json::from),
        Value::BigUnsigned(v) => v.map_or(Json::Null, uint_json),
        // Through the shortest decimal form: `f64::from(0.1_f32)` is
        // 0.10000000149011612.
        Value::Float(v) => v
            .and_then(|f| f.to_string().parse::<f64>().ok())
            .and_then(serde_json::Number::from_f64)
            .map_or(Json::Null, Json::Number),
        Value::Double(v) => v
            .and_then(serde_json::Number::from_f64)
            .map_or(Json::Null, Json::Number),
        Value::String(v) => v.as_ref().map_or(Json::Null, |s| Json::String(s.clone())),
        Value::Char(v) => v.map_or(Json::Null, |c| Json::String(c.to_string())),
        Value::Enum(OptionEnum::Some(e)) => Json::String(e.value.to_string()),
        Value::Enum(OptionEnum::None(_)) => Json::Null,
        Value::Bytes(v) => v
            .as_ref()
            .map_or(Json::Null, |b| Json::String(format!("<{} bytes>", b.len()))),
        Value::Json(v) => v.as_deref().cloned().unwrap_or(Json::Null),
        Value::ChronoDate(v) => v.map_or(Json::Null, |d| {
            Json::String(d.format("%Y-%m-%d").to_string())
        }),
        Value::ChronoTime(v) => v.map_or(Json::Null, |t| {
            Json::String(t.format("%H:%M:%S%.f").to_string())
        }),
        Value::ChronoDateTime(v) => v.map_or(Json::Null, |d| {
            Json::String(d.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
        }),
        Value::ChronoDateTimeUtc(v) => v.map_or(Json::Null, |d| Json::String(d.to_rfc3339())),
        Value::ChronoDateTimeLocal(v) => v.map_or(Json::Null, |d| Json::String(d.to_rfc3339())),
        Value::ChronoDateTimeWithTimeZone(v) => {
            v.map_or(Json::Null, |d| Json::String(d.to_rfc3339()))
        }
        Value::Uuid(v) => v.map_or(Json::Null, |u| Json::String(u.to_string())),
        Value::Decimal(v) => v.map_or(Json::Null, |d| Json::String(d.to_string())),
        #[allow(unreachable_patterns)]
        other => sea_orm::sea_query::sea_value_to_json_value(other),
    }
}

/// Render a value as plain text (record keys, labels, CSV cells).
pub(crate) fn to_text(value: &Value) -> String {
    json_to_text(&to_json(value))
}

/// Render JSON as plain text: strings unquoted, null empty.
pub(crate) fn json_to_text(json: &Json) -> String {
    match json {
        Json::Null => String::new(),
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Whether a value is SQL `NULL`.
pub(crate) fn is_null(value: &Value) -> bool {
    to_json(value).is_null() && !matches!(value, Value::Json(Some(_)))
}

fn input_text(input: &Json) -> Option<String> {
    match input {
        Json::String(s) => Some(s.trim().to_string()),
        Json::Number(n) => Some(n.to_string()),
        Json::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn integer_candidates(col_type: &ColumnType, input: &Json) -> Result<Vec<Value>, String> {
    let n: i128 = match input {
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                i128::from(i)
            } else if let Some(u) = n.as_u64() {
                i128::from(u)
            } else {
                return Err("must be an integer".to_string());
            }
        }
        Json::String(s) => s
            .trim()
            .parse::<i128>()
            .map_err(|_| "must be an integer".to_string())?,
        _ => return Err("must be an integer".to_string()),
    };
    let preferred: &[u8] = match col_type {
        ColumnType::TinyInteger => &[0, 1, 2, 3],
        ColumnType::SmallInteger => &[1, 2, 3, 0],
        ColumnType::BigInteger => &[3, 2, 1, 0],
        ColumnType::TinyUnsigned => &[4, 5, 6, 7],
        ColumnType::SmallUnsigned => &[5, 6, 7, 4],
        ColumnType::Unsigned => &[6, 7, 5, 4],
        ColumnType::BigUnsigned => &[7, 6, 5, 4],
        _ => &[2, 3, 1, 0],
    };
    let mut order: Vec<u8> = preferred.to_vec();
    order.extend((0..8).filter(|i| !preferred.contains(i)));
    let out: Vec<Value> = order
        .into_iter()
        .filter_map(|kind| match kind {
            0 => i8::try_from(n).ok().map(|v| Value::TinyInt(Some(v))),
            1 => i16::try_from(n).ok().map(|v| Value::SmallInt(Some(v))),
            2 => i32::try_from(n).ok().map(|v| Value::Int(Some(v))),
            3 => i64::try_from(n).ok().map(|v| Value::BigInt(Some(v))),
            4 => u8::try_from(n).ok().map(|v| Value::TinyUnsigned(Some(v))),
            5 => u16::try_from(n).ok().map(|v| Value::SmallUnsigned(Some(v))),
            6 => u32::try_from(n).ok().map(|v| Value::Unsigned(Some(v))),
            _ => u64::try_from(n).ok().map(|v| Value::BigUnsigned(Some(v))),
        })
        .collect();
    if out.is_empty() {
        return Err("is out of range".to_string());
    }
    Ok(out)
}

fn float_value(input: &Json) -> Result<f64, String> {
    match input {
        Json::Number(n) => n.as_f64().ok_or_else(|| "must be a number".to_string()),
        Json::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|f| f.is_finite())
            .ok_or_else(|| "must be a number".to_string()),
        _ => Err("must be a number".to_string()),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn float_candidates(prefer_single: bool, input: &Json) -> Result<Vec<Value>, String> {
    let f = float_value(input)?;
    let double = Value::Double(Some(f));
    let single = Value::Float(Some(f as f32));
    Ok(if prefer_single {
        vec![single, double]
    } else {
        vec![double, single]
    })
}

fn decimal_candidates(input: &Json) -> Result<Vec<Value>, String> {
    let text = input_text(input)
        .filter(|_| !matches!(input, Json::Bool(_)))
        .ok_or_else(|| "must be a decimal number".to_string())?;
    let decimal = Decimal::from_str(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .map_err(|_| "must be a decimal number".to_string())?;
    let mut out = vec![Value::Decimal(Some(decimal))];
    out.extend(float_candidates(false, input)?);
    Ok(out)
}

fn bool_value(input: &Json) -> Result<bool, String> {
    match input {
        Json::Bool(b) => Ok(*b),
        Json::Number(n) if n.as_i64() == Some(0) => Ok(false),
        Json::Number(n) if n.as_i64() == Some(1) => Ok(true),
        Json::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "on" | "yes" | "t" => Ok(true),
            "false" | "0" | "off" | "no" | "f" => Ok(false),
            _ => Err("must be a boolean".to_string()),
        },
        _ => Err("must be a boolean".to_string()),
    }
}

const NAIVE_DATETIME_FORMATS: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
];

fn parse_naive_datetime(text: &str) -> Option<ChronoDateTime> {
    NAIVE_DATETIME_FORMATS
        .iter()
        .find_map(|fmt| ChronoDateTime::parse_from_str(text, fmt).ok())
        .or_else(|| {
            ChronoDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })
}

fn parse_datetime_tz(text: &str) -> Option<ChronoDateTimeWithTimeZone> {
    ChronoDateTimeWithTimeZone::parse_from_rfc3339(text)
        .ok()
        .or_else(|| {
            [
                "%Y-%m-%d %H:%M:%S%.f%:z",
                "%Y-%m-%d %H:%M:%S%.f %:z",
                "%Y-%m-%dT%H:%M%:z",
            ]
            .iter()
            .find_map(|fmt| ChronoDateTimeWithTimeZone::parse_from_str(text, fmt).ok())
        })
        .or_else(|| parse_naive_datetime(text).map(|n| n.and_utc().fixed_offset()))
}

fn text_input(input: &Json, what: &str) -> Result<String, String> {
    match input {
        Json::String(s) => Ok(s.trim().to_string()),
        _ => Err(format!("must be {what}")),
    }
}

fn datetime_candidates(with_tz: bool, input: &Json) -> Result<Vec<Value>, String> {
    let text = text_input(input, "a date-time")?;
    let dt = parse_datetime_tz(&text).ok_or_else(|| "must be a date-time".to_string())?;
    let utc: ChronoDateTimeUtc = dt.into();
    let local: ChronoDateTimeLocal = dt.into();
    // Inputs without an offset are parsed as UTC, so this is the input as
    // written for naive inputs and the UTC instant for zoned ones.
    let naive = dt.naive_utc();
    let zoned = vec![
        Value::ChronoDateTimeWithTimeZone(Some(dt)),
        Value::ChronoDateTimeUtc(Some(utc)),
        Value::ChronoDateTimeLocal(Some(local)),
    ];
    let naive = Value::ChronoDateTime(Some(naive));
    Ok(if with_tz {
        let mut out = zoned;
        out.push(naive);
        out
    } else {
        let mut out = vec![naive];
        out.extend(zoned);
        out
    })
}

fn enum_candidates(name: &str, variants: &[String], input: &Json) -> Result<Vec<Value>, String> {
    let text = input_text(input).ok_or_else(|| "must be a string".to_string())?;
    if !variants.is_empty() && !variants.contains(&text) {
        return Err(format!("must be one of: {}", variants.join(", ")));
    }
    Ok(vec![
        Value::Enum(OptionEnum::Some(Box::new(Enum {
            type_name: name.to_string().into(),
            value: text.clone().into(),
        }))),
        Value::String(Some(text)),
    ])
}

/// Candidate values for a non-null JSON input.
fn non_null_candidates(col_type: &ColumnType, input: &Json) -> Result<Vec<Value>, String> {
    match col_type {
        ColumnType::TinyInteger
        | ColumnType::SmallInteger
        | ColumnType::Integer
        | ColumnType::BigInteger
        | ColumnType::TinyUnsigned
        | ColumnType::SmallUnsigned
        | ColumnType::Unsigned
        | ColumnType::BigUnsigned
        | ColumnType::Year => integer_candidates(col_type, input),
        ColumnType::Float => float_candidates(true, input),
        ColumnType::Double => float_candidates(false, input),
        ColumnType::Decimal(_) | ColumnType::Money(_) => decimal_candidates(input),
        ColumnType::Boolean => Ok(vec![Value::Bool(Some(bool_value(input)?))]),
        ColumnType::Date => {
            let text = text_input(input, "a date")?;
            let date = ChronoDate::parse_from_str(&text, "%Y-%m-%d")
                .ok()
                .or_else(|| parse_datetime_tz(&text).map(|d| d.date_naive()))
                .ok_or_else(|| "must be a date (YYYY-MM-DD)".to_string())?;
            Ok(vec![Value::ChronoDate(Some(date))])
        }
        ColumnType::Time => {
            let text = text_input(input, "a time")?;
            let time = ["%H:%M:%S%.f", "%H:%M"]
                .iter()
                .find_map(|fmt| ChronoTime::parse_from_str(&text, fmt).ok())
                .ok_or_else(|| "must be a time (HH:MM:SS)".to_string())?;
            Ok(vec![Value::ChronoTime(Some(time))])
        }
        ColumnType::DateTime | ColumnType::Timestamp => datetime_candidates(false, input),
        ColumnType::TimestampWithTimeZone => datetime_candidates(true, input),
        ColumnType::Uuid => {
            let text = text_input(input, "a UUID")?;
            let uuid = Uuid::parse_str(&text).map_err(|_| "must be a UUID".to_string())?;
            Ok(vec![Value::Uuid(Some(uuid)), Value::String(Some(text))])
        }
        ColumnType::Json | ColumnType::JsonBinary => {
            Ok(vec![Value::Json(Some(Box::new(input.clone())))])
        }
        ColumnType::Enum { name, variants } => {
            let variants: Vec<String> = variants.iter().map(ToString::to_string).collect();
            enum_candidates(&name.to_string(), &variants, input)
        }
        ColumnType::Binary(_) | ColumnType::VarBinary(_) | ColumnType::Blob => {
            let text = text_input(input, "text")?;
            Ok(vec![Value::Bytes(Some(text.into_bytes()))])
        }
        ColumnType::Array(_) | ColumnType::Vector(_) => {
            Err("editing this column type is not supported".to_string())
        }
        ColumnType::Char(Some(1)) => {
            let text = input_text(input).ok_or_else(|| "must be text".to_string())?;
            let mut out = Vec::new();
            let mut chars = text.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                out.push(Value::Char(Some(c)));
            }
            out.push(Value::String(Some(text)));
            Ok(out)
        }
        _ => {
            let text = match input {
                Json::String(s) => s.clone(),
                Json::Number(_) | Json::Bool(_) => input.to_string(),
                _ => return Err("must be text".to_string()),
            };
            Ok(vec![Value::String(Some(text))])
        }
    }
}

/// A representative non-null JSON value for a column type, used to derive the
/// typed `NULL` candidates.
fn sample_input(col_type: &ColumnType) -> Json {
    match col_type {
        ColumnType::TinyInteger
        | ColumnType::SmallInteger
        | ColumnType::Integer
        | ColumnType::BigInteger
        | ColumnType::TinyUnsigned
        | ColumnType::SmallUnsigned
        | ColumnType::Unsigned
        | ColumnType::BigUnsigned
        | ColumnType::Year
        | ColumnType::Float
        | ColumnType::Double
        | ColumnType::Decimal(_)
        | ColumnType::Money(_) => Json::from(0),
        ColumnType::Boolean => Json::Bool(false),
        ColumnType::Date => Json::from("1970-01-01"),
        ColumnType::Time => Json::from("00:00:00"),
        ColumnType::DateTime | ColumnType::Timestamp | ColumnType::TimestampWithTimeZone => {
            Json::from("1970-01-01T00:00:00Z")
        }
        ColumnType::Uuid => Json::from("00000000-0000-0000-0000-000000000000"),
        ColumnType::Enum { variants, .. } => Json::from(
            variants
                .first()
                .map(ToString::to_string)
                .unwrap_or_default(),
        ),
        ColumnType::Char(Some(1)) => Json::from("a"),
        _ => Json::from(""),
    }
}

/// Candidate typed values for `input` on a column of `col_type`, most likely
/// first.
pub(crate) fn candidates(col_type: &ColumnType, input: &Json) -> Result<Vec<Value>, String> {
    if input.is_null() {
        if matches!(col_type, ColumnType::Json | ColumnType::JsonBinary) {
            return Ok(vec![
                Value::Json(None),
                Value::Json(Some(Box::new(Json::Null))),
            ]);
        }
        if matches!(col_type, ColumnType::Array(_) | ColumnType::Vector(_)) {
            return Err("editing this column type is not supported".to_string());
        }
        let samples = non_null_candidates(col_type, &sample_input(col_type))?;
        return Ok(samples.iter().map(Value::as_null).collect());
    }
    non_null_candidates(col_type, input)
}

/// Convert `input` into the value `column` accepts, using a scratch active
/// model to probe which candidate matches the field's Rust type.
pub(crate) fn convert<A>(
    column: <A::Entity as EntityTrait>::Column,
    input: &Json,
) -> Result<Value, String>
where
    A: ActiveModelTrait,
{
    let col_type = column.def().get_column_type().clone();
    let mut scratch = A::default();
    for candidate in candidates(&col_type, input)? {
        if scratch.try_set(column, candidate).is_ok()
            && let Some(value) = scratch.get(column).into_value()
        {
            return Ok(value);
        }
    }
    Err(if input.is_null() {
        "cannot be empty".to_string()
    } else {
        "has an invalid value".to_string()
    })
}

/// Characters escaped in each part of a record key.
const KEY_PART: &AsciiSet = &CONTROLS.add(b'%').add(b',');

/// Encode primary-key values into a single URL-friendly record key.
pub(crate) fn encode_key(values: &[Value]) -> String {
    values
        .iter()
        .map(|v| utf8_percent_encode(&to_text(v), KEY_PART).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Split a record key produced by [`encode_key`] into its parts.
pub(crate) fn decode_key(key: &str) -> Vec<String> {
    key.split(',')
        .map(|part| percent_decode_str(part).decode_utf8_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::{Alias, IntoIden, StringLen};
    use serde_json::json;

    fn ct_enum() -> ColumnType {
        ColumnType::Enum {
            name: Alias::new("status").into_iden(),
            variants: vec![
                Alias::new("draft").into_iden(),
                Alias::new("published").into_iden(),
            ],
        }
    }

    #[test]
    fn json_round_trip_of_scalars() {
        assert_eq!(to_json(&Value::Bool(Some(true))), json!(true));
        assert_eq!(to_json(&Value::TinyInt(Some(-1))), json!(-1));
        assert_eq!(to_json(&Value::SmallInt(Some(2))), json!(2));
        assert_eq!(to_json(&Value::Int(Some(3))), json!(3));
        assert_eq!(to_json(&Value::BigInt(Some(4))), json!(4));
        assert_eq!(to_json(&Value::TinyUnsigned(Some(5))), json!(5));
        assert_eq!(to_json(&Value::SmallUnsigned(Some(6))), json!(6));
        assert_eq!(to_json(&Value::Unsigned(Some(7))), json!(7));
        assert_eq!(to_json(&Value::BigUnsigned(Some(8))), json!(8));
        assert_eq!(to_json(&Value::Float(Some(1.5))), json!(1.5));
        assert_eq!(to_json(&Value::Double(Some(2.5))), json!(2.5));
        assert_eq!(to_json(&Value::Double(Some(f64::NAN))), Json::Null);
        assert_eq!(to_json(&Value::String(Some("a".into()))), json!("a"));
        assert_eq!(to_json(&Value::Char(Some('c'))), json!("c"));
        assert_eq!(to_json(&Value::Bytes(Some(vec![1, 2]))), json!("<2 bytes>"));
        assert_eq!(
            to_json(&Value::Json(Some(Box::new(json!({"a": 1}))))),
            json!({"a": 1})
        );
        assert_eq!(to_json(&Value::Int(None)), Json::Null);
        assert_eq!(to_json(&Value::Json(None)), Json::Null);
    }

    #[test]
    fn integers_javascript_would_round_are_strings() {
        let max = (1_i64 << 53) - 1;
        assert_eq!(to_json(&Value::BigInt(Some(max))), json!(max));
        assert_eq!(to_json(&Value::BigInt(Some(-max))), json!(-max));
        assert_eq!(
            to_json(&Value::BigInt(Some(max + 2))),
            json!("9007199254740993")
        );
        assert_eq!(
            to_json(&Value::BigInt(Some(i64::MIN))),
            json!(i64::MIN.to_string())
        );
        assert_eq!(to_json(&Value::BigUnsigned(Some(42))), json!(42));
        assert_eq!(
            to_json(&Value::BigUnsigned(Some(u64::MAX))),
            json!(u64::MAX.to_string())
        );
        assert_eq!(to_text(&Value::BigInt(Some(max + 2))), "9007199254740993");
        let back = candidates(&ColumnType::BigInteger, &json!("9007199254740993")).unwrap();
        assert_eq!(back[0], Value::BigInt(Some(max + 2)));
    }

    #[test]
    fn single_precision_floats_keep_their_short_form() {
        assert_eq!(to_json(&Value::Float(Some(0.1))), json!(0.1));
        assert_eq!(to_text(&Value::Float(Some(0.1))), "0.1");
        assert_eq!(to_json(&Value::Float(Some(f32::NAN))), Json::Null);
    }

    #[test]
    fn json_of_temporal_and_misc_values() {
        let date = ChronoDate::from_ymd_opt(2024, 2, 29).unwrap();
        assert_eq!(to_json(&Value::ChronoDate(Some(date))), json!("2024-02-29"));
        let time = ChronoTime::from_hms_opt(13, 5, 9).unwrap();
        assert_eq!(to_json(&Value::ChronoTime(Some(time))), json!("13:05:09"));
        let naive = date.and_time(time);
        assert_eq!(
            to_json(&Value::ChronoDateTime(Some(naive))),
            json!("2024-02-29T13:05:09")
        );
        let utc = naive.and_utc();
        assert_eq!(
            to_json(&Value::ChronoDateTimeUtc(Some(utc))),
            json!("2024-02-29T13:05:09+00:00")
        );
        assert_eq!(
            to_json(&Value::ChronoDateTimeWithTimeZone(Some(utc.fixed_offset()))),
            json!("2024-02-29T13:05:09+00:00")
        );
        let local: ChronoDateTimeLocal = utc.into();
        assert!(to_json(&Value::ChronoDateTimeLocal(Some(local))).is_string());
        let uuid = Uuid::nil();
        assert_eq!(
            to_json(&Value::Uuid(Some(uuid))),
            json!("00000000-0000-0000-0000-000000000000")
        );
        assert_eq!(
            to_json(&Value::Decimal(Some(Decimal::new(1050, 2)))),
            json!("10.50")
        );
        let e = Value::Enum(OptionEnum::Some(Box::new(Enum {
            type_name: "status".to_string().into(),
            value: "draft".into(),
        })));
        assert_eq!(to_json(&e), json!("draft"));
        assert_eq!(
            to_json(&Value::Enum(OptionEnum::None("status".to_string().into()))),
            Json::Null
        );
    }

    #[test]
    fn text_rendering() {
        assert_eq!(to_text(&Value::String(Some("x".into()))), "x");
        assert_eq!(to_text(&Value::Int(Some(5))), "5");
        assert_eq!(to_text(&Value::Int(None)), "");
        assert_eq!(json_to_text(&json!(true)), "true");
        assert!(is_null(&Value::Int(None)));
        assert!(!is_null(&Value::Json(Some(Box::new(Json::Null)))));
        assert!(!is_null(&Value::Int(Some(0))));
    }

    #[test]
    fn integer_inputs() {
        let c = candidates(&ColumnType::Integer, &json!(42)).unwrap();
        assert_eq!(c[0], Value::Int(Some(42)));
        let c = candidates(&ColumnType::BigInteger, &json!("42")).unwrap();
        assert_eq!(c[0], Value::BigInt(Some(42)));
        let c = candidates(&ColumnType::TinyInteger, &json!(300)).unwrap();
        assert_eq!(c[0], Value::SmallInt(Some(300)));
        let c = candidates(&ColumnType::SmallInteger, &json!(1)).unwrap();
        assert_eq!(c[0], Value::SmallInt(Some(1)));
        let c = candidates(&ColumnType::TinyUnsigned, &json!(1)).unwrap();
        assert_eq!(c[0], Value::TinyUnsigned(Some(1)));
        let c = candidates(&ColumnType::SmallUnsigned, &json!(1)).unwrap();
        assert_eq!(c[0], Value::SmallUnsigned(Some(1)));
        let c = candidates(&ColumnType::Unsigned, &json!(1)).unwrap();
        assert_eq!(c[0], Value::Unsigned(Some(1)));
        let c = candidates(&ColumnType::BigUnsigned, &json!(u64::MAX)).unwrap();
        assert_eq!(c, vec![Value::BigUnsigned(Some(u64::MAX))]);
        assert!(candidates(&ColumnType::Integer, &json!(1.5)).is_err());
        assert!(candidates(&ColumnType::Integer, &json!("x")).is_err());
        assert!(candidates(&ColumnType::Integer, &json!(true)).is_err());
        let huge = json!("999999999999999999999999");
        assert_eq!(
            candidates(&ColumnType::Integer, &huge).unwrap_err(),
            "is out of range"
        );
    }

    #[test]
    fn float_decimal_bool_inputs() {
        assert_eq!(
            candidates(&ColumnType::Double, &json!("2.5")).unwrap()[0],
            Value::Double(Some(2.5))
        );
        assert_eq!(
            candidates(&ColumnType::Float, &json!(2.5)).unwrap()[0],
            Value::Float(Some(2.5))
        );
        assert!(candidates(&ColumnType::Double, &json!("nan")).is_err());
        assert!(candidates(&ColumnType::Double, &json!([])).is_err());
        assert_eq!(
            candidates(&ColumnType::Decimal(None), &json!("10.25")).unwrap()[0],
            Value::Decimal(Some(Decimal::new(1025, 2)))
        );
        assert_eq!(
            candidates(&ColumnType::Money(None), &json!(3)).unwrap()[0],
            Value::Decimal(Some(Decimal::new(3, 0)))
        );
        assert_eq!(
            candidates(&ColumnType::Decimal(None), &json!("1e2")).unwrap()[0],
            Value::Decimal(Some(Decimal::new(100, 0)))
        );
        assert!(candidates(&ColumnType::Decimal(None), &json!(true)).is_err());
        assert!(candidates(&ColumnType::Decimal(None), &json!("abc")).is_err());
        for (input, expected) in [
            (json!(true), true),
            (json!("on"), true),
            (json!("0"), false),
            (json!(1), true),
            (json!(0), false),
            (json!("False"), false),
        ] {
            assert_eq!(
                candidates(&ColumnType::Boolean, &input).unwrap(),
                vec![Value::Bool(Some(expected))]
            );
        }
        assert!(candidates(&ColumnType::Boolean, &json!("maybe")).is_err());
        assert!(candidates(&ColumnType::Boolean, &json!(2)).is_err());
    }

    #[test]
    fn temporal_inputs() {
        let date = candidates(&ColumnType::Date, &json!("2024-01-31")).unwrap();
        assert_eq!(
            date,
            vec![Value::ChronoDate(ChronoDate::from_ymd_opt(2024, 1, 31))]
        );
        let date = candidates(&ColumnType::Date, &json!("2024-01-31T10:00:00Z")).unwrap();
        assert_eq!(
            date,
            vec![Value::ChronoDate(ChronoDate::from_ymd_opt(2024, 1, 31))]
        );
        assert!(candidates(&ColumnType::Date, &json!("31/01/2024")).is_err());
        assert!(candidates(&ColumnType::Date, &json!(5)).is_err());

        let time = candidates(&ColumnType::Time, &json!("10:30")).unwrap();
        assert_eq!(
            time,
            vec![Value::ChronoTime(ChronoTime::from_hms_opt(10, 30, 0))]
        );
        assert!(candidates(&ColumnType::Time, &json!("25:00")).is_err());

        let naive = candidates(&ColumnType::DateTime, &json!("2024-01-31T10:30")).unwrap();
        let expected = ChronoDate::from_ymd_opt(2024, 1, 31)
            .unwrap()
            .and_hms_opt(10, 30, 0)
            .unwrap();
        assert_eq!(naive[0], Value::ChronoDateTime(Some(expected)));
        let naive =
            candidates(&ColumnType::Timestamp, &json!("2024-01-31T12:30:00+02:00")).unwrap();
        assert_eq!(naive[0], Value::ChronoDateTime(Some(expected)));

        let tz = candidates(
            &ColumnType::TimestampWithTimeZone,
            &json!("2024-01-31T10:30:00+02:00"),
        )
        .unwrap();
        let Value::ChronoDateTimeWithTimeZone(Some(dt)) = &tz[0] else {
            panic!("expected tz value");
        };
        assert_eq!(dt.to_rfc3339(), "2024-01-31T10:30:00+02:00");
        assert!(matches!(tz[1], Value::ChronoDateTimeUtc(Some(_))));
        assert!(matches!(tz[2], Value::ChronoDateTimeLocal(Some(_))));
        let tz = candidates(
            &ColumnType::TimestampWithTimeZone,
            &json!("2024-01-31 10:30"),
        )
        .unwrap();
        let Value::ChronoDateTimeWithTimeZone(Some(dt)) = &tz[0] else {
            panic!("expected tz value");
        };
        assert_eq!(dt.to_rfc3339(), "2024-01-31T10:30:00+00:00");
        let tz = candidates(
            &ColumnType::TimestampWithTimeZone,
            &json!("2024-01-31 10:30:00+01:00"),
        )
        .unwrap();
        assert!(matches!(tz[0], Value::ChronoDateTimeWithTimeZone(Some(_))));
        assert!(candidates(&ColumnType::TimestampWithTimeZone, &json!("soon")).is_err());
    }

    #[test]
    fn string_uuid_json_enum_binary_inputs() {
        assert_eq!(
            candidates(&ColumnType::String(StringLen::None), &json!("hi")).unwrap(),
            vec![Value::String(Some("hi".into()))]
        );
        assert_eq!(
            candidates(&ColumnType::Text, &json!(12)).unwrap(),
            vec![Value::String(Some("12".into()))]
        );
        assert!(candidates(&ColumnType::Text, &json!({"a": 1})).is_err());
        let c = candidates(&ColumnType::Char(Some(1)), &json!("x")).unwrap();
        assert_eq!(c[0], Value::Char(Some('x')));
        let c = candidates(&ColumnType::Char(Some(1)), &json!("xy")).unwrap();
        assert_eq!(c, vec![Value::String(Some("xy".into()))]);
        assert!(candidates(&ColumnType::Char(Some(1)), &json!([])).is_err());

        let id = "67e55044-10b1-426f-9247-bb680e5fe0c8";
        let c = candidates(&ColumnType::Uuid, &json!(id)).unwrap();
        assert_eq!(c[0], Value::Uuid(Some(Uuid::parse_str(id).unwrap())));
        assert!(candidates(&ColumnType::Uuid, &json!("nope")).is_err());

        let c = candidates(&ColumnType::JsonBinary, &json!({"a": [1]})).unwrap();
        assert_eq!(c, vec![Value::Json(Some(Box::new(json!({"a": [1]}))))]);

        let c = candidates(&ct_enum(), &json!("draft")).unwrap();
        assert!(matches!(&c[0], Value::Enum(OptionEnum::Some(e)) if e.value == "draft"));
        assert_eq!(c[1], Value::String(Some("draft".into())));
        assert_eq!(
            candidates(&ct_enum(), &json!("other")).unwrap_err(),
            "must be one of: draft, published"
        );

        let c = candidates(&ColumnType::Blob, &json!("raw")).unwrap();
        assert_eq!(c, vec![Value::Bytes(Some(b"raw".to_vec()))]);
        let arr = ColumnType::Array(sea_orm::sea_query::RcOrArc::new(ColumnType::Integer));
        assert!(candidates(&arr, &json!([1])).is_err());
        assert!(candidates(&arr, &Json::Null).is_err());
        assert_eq!(
            candidates(&ColumnType::Inet, &json!("127.0.0.1")).unwrap(),
            vec![Value::String(Some("127.0.0.1".into()))]
        );
    }

    #[test]
    fn null_inputs_produce_typed_nulls() {
        assert_eq!(
            candidates(&ColumnType::Integer, &Json::Null).unwrap()[0],
            Value::Int(None)
        );
        assert_eq!(
            candidates(&ColumnType::Json, &Json::Null).unwrap(),
            vec![Value::Json(None), Value::Json(Some(Box::new(Json::Null)))]
        );
        assert!(matches!(
            candidates(&ct_enum(), &Json::Null).unwrap()[0],
            Value::Enum(OptionEnum::None(_))
        ));
        for ct in [
            ColumnType::Boolean,
            ColumnType::Date,
            ColumnType::Time,
            ColumnType::DateTime,
            ColumnType::TimestampWithTimeZone,
            ColumnType::Uuid,
            ColumnType::Decimal(None),
            ColumnType::Text,
            ColumnType::Char(Some(1)),
            ColumnType::Double,
        ] {
            let c = candidates(&ct, &Json::Null).unwrap();
            assert!(c.iter().all(is_null), "{ct:?}");
        }
    }

    #[test]
    fn keys_round_trip() {
        let key = encode_key(&[Value::Int(Some(5))]);
        assert_eq!(key, "5");
        assert_eq!(decode_key(&key), vec!["5"]);
        let key = encode_key(&[
            Value::String(Some("a,b%c".into())),
            Value::String(Some("%2C".into())),
        ]);
        assert_eq!(key, "a%2Cb%25c,%252C");
        assert_eq!(decode_key(&key), vec!["a,b%c", "%2C"]);
        assert_eq!(decode_key("50%"), vec!["50%"]);
        assert_eq!(decode_key("a%2cb"), vec!["a,b"]);
        assert_eq!(decode_key("%zz"), vec!["%zz"]);
    }
}

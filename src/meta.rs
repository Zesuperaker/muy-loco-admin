//! The schema the admin UI renders, derived from `SeaORM` entity metadata.

use sea_orm::sea_query::{ColumnType, StringLen};
use serde::Serialize;

/// How a field is rendered and edited in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    String,
    Text,
    Integer,
    Float,
    Decimal,
    Boolean,
    Date,
    Time,
    DateTime,
    Uuid,
    Json,
    Enum,
    Binary,
    Array,
    Other,
}

impl FieldKind {
    pub(crate) fn from_column_type(col_type: &ColumnType) -> Self {
        match col_type {
            ColumnType::Char(_) | ColumnType::String(_) => Self::String,
            ColumnType::Text => Self::Text,
            ColumnType::TinyInteger
            | ColumnType::SmallInteger
            | ColumnType::Integer
            | ColumnType::BigInteger
            | ColumnType::TinyUnsigned
            | ColumnType::SmallUnsigned
            | ColumnType::Unsigned
            | ColumnType::BigUnsigned
            | ColumnType::Year => Self::Integer,
            ColumnType::Float | ColumnType::Double => Self::Float,
            ColumnType::Decimal(_) | ColumnType::Money(_) => Self::Decimal,
            ColumnType::Boolean => Self::Boolean,
            ColumnType::Date => Self::Date,
            ColumnType::Time => Self::Time,
            ColumnType::DateTime | ColumnType::Timestamp | ColumnType::TimestampWithTimeZone => {
                Self::DateTime
            }
            ColumnType::Uuid => Self::Uuid,
            ColumnType::Json | ColumnType::JsonBinary => Self::Json,
            ColumnType::Enum { .. } => Self::Enum,
            ColumnType::Binary(_) | ColumnType::VarBinary(_) | ColumnType::Blob => Self::Binary,
            ColumnType::Array(_) | ColumnType::Vector(_) => Self::Array,
            _ => Self::Other,
        }
    }

    /// Kinds that can be searched with a text `LIKE` without casting.
    pub(crate) fn is_textual(self) -> bool {
        matches!(self, Self::String | Self::Text)
    }

    /// Kinds that can never be edited through the admin.
    pub(crate) fn is_read_only(self) -> bool {
        matches!(self, Self::Binary | Self::Array)
    }
}

pub(crate) fn max_length(col_type: &ColumnType) -> Option<u32> {
    match col_type {
        ColumnType::Char(len) => *len,
        ColumnType::String(StringLen::N(n)) => Some(*n),
        _ => None,
    }
}

pub(crate) fn enum_choices(col_type: &ColumnType) -> Vec<String> {
    match col_type {
        ColumnType::Enum { variants, .. } => variants.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Turn `created_at` / `blog_posts` into `Created at` / `Blog posts`.
#[must_use]
pub fn humanize(name: &str) -> String {
    let spaced = name.replace(['_', '-'], " ");
    let trimmed = spaced.trim();
    let trimmed = trimmed.strip_suffix(" id").unwrap_or(trimmed);
    let mut chars = trimmed.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// A foreign key from this field to another table.
#[derive(Debug, Clone, Serialize)]
pub struct RelationMeta {
    /// The referenced table.
    pub table: String,
    /// The referenced column.
    pub column: String,
    /// The registered admin model for `table`, if any.
    pub model: Option<String>,
}

/// A field of a model as exposed to the UI.
#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct FieldMeta {
    pub name: String,
    pub label: String,
    pub kind: FieldKind,
    pub nullable: bool,
    pub primary_key: bool,
    pub auto_increment: bool,
    pub readonly: bool,
    pub unique: bool,
    /// Whether a date-time carries a time zone.
    pub timezone: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<RelationMeta>,
}

/// Another model whose foreign key points at this one.
#[derive(Debug, Clone, Serialize)]
pub struct RelatedMeta {
    pub model: String,
    pub field: String,
    pub label: String,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

/// Default ordering of a list.
#[derive(Debug, Clone, Serialize)]
pub struct Ordering {
    pub field: String,
    pub direction: SortDirection,
}

/// A custom bulk action.
#[derive(Debug, Clone, Serialize)]
pub struct ActionMeta {
    pub name: String,
    pub label: String,
}

/// Operations a model supports, before per-user permissions are applied.
#[derive(Debug, Clone, Copy, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    pub view: bool,
    pub create: bool,
    pub update: bool,
    pub delete: bool,
    pub export: bool,
}

/// Everything the UI needs to render a model.
#[derive(Debug, Clone, Serialize)]
pub struct ModelMeta {
    pub name: String,
    pub label: String,
    pub table: String,
    pub primary_key: Vec<String>,
    pub fields: Vec<FieldMeta>,
    pub list_display: Vec<String>,
    pub search_fields: Vec<String>,
    pub list_filter: Vec<String>,
    pub ordering: Ordering,
    pub per_page: u64,
    pub display_field: Option<String>,
    pub actions: Vec<ActionMeta>,
    pub related: Vec<RelatedMeta>,
    pub capabilities: Capabilities,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::{Alias, IntoIden, RcOrArc};

    #[test]
    fn humanizes_names() {
        assert_eq!(humanize("created_at"), "Created at");
        assert_eq!(humanize("blog_posts"), "Blog posts");
        assert_eq!(humanize("user_id"), "User");
        assert_eq!(humanize("id"), "Id");
        assert_eq!(humanize(""), "");
        assert_eq!(humanize("über-cool"), "Über cool");
    }

    #[test]
    fn kinds_from_column_types() {
        use FieldKind as K;
        let cases = [
            (ColumnType::Char(Some(2)), K::String),
            (ColumnType::String(StringLen::N(10)), K::String),
            (ColumnType::Text, K::Text),
            (ColumnType::Integer, K::Integer),
            (ColumnType::BigUnsigned, K::Integer),
            (ColumnType::Year, K::Integer),
            (ColumnType::Double, K::Float),
            (ColumnType::Money(None), K::Decimal),
            (ColumnType::Boolean, K::Boolean),
            (ColumnType::Date, K::Date),
            (ColumnType::Time, K::Time),
            (ColumnType::Timestamp, K::DateTime),
            (ColumnType::TimestampWithTimeZone, K::DateTime),
            (ColumnType::Uuid, K::Uuid),
            (ColumnType::JsonBinary, K::Json),
            (
                ColumnType::Enum {
                    name: Alias::new("e").into_iden(),
                    variants: vec![],
                },
                K::Enum,
            ),
            (ColumnType::Blob, K::Binary),
            (ColumnType::Array(RcOrArc::new(ColumnType::Text)), K::Array),
            (ColumnType::Inet, K::Other),
        ];
        for (ct, kind) in cases {
            assert_eq!(FieldKind::from_column_type(&ct), kind, "{ct:?}");
        }
        assert!(K::Text.is_textual());
        assert!(!K::Integer.is_textual());
        assert!(K::Binary.is_read_only());
        assert!(!K::Json.is_read_only());
    }

    #[test]
    fn lengths_and_choices() {
        assert_eq!(max_length(&ColumnType::String(StringLen::N(5))), Some(5));
        assert_eq!(max_length(&ColumnType::Char(Some(3))), Some(3));
        assert_eq!(max_length(&ColumnType::Text), None);
        let ct = ColumnType::Enum {
            name: Alias::new("s").into_iden(),
            variants: vec![Alias::new("a").into_iden(), Alias::new("b").into_iden()],
        };
        assert_eq!(enum_choices(&ct), vec!["a", "b"]);
        assert!(enum_choices(&ColumnType::Text).is_empty());
    }
}

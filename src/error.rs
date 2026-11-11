//! Errors returned by the admin API.

use std::collections::BTreeMap;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{DbErr, SqlErr};
use serde::{Deserialize, Serialize};

/// Field name to error messages.
pub type FieldErrors = BTreeMap<String, Vec<String>>;

/// An error produced while serving an admin request.
#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    /// No valid credentials were presented (401).
    #[error("authentication required")]
    Unauthenticated,
    /// Credentials are valid but the operation is not permitted (403).
    #[error("you do not have permission to perform this action")]
    Forbidden,
    /// The model or record does not exist (404).
    #[error("not found")]
    NotFound,
    /// The request is malformed (400).
    #[error("{0}")]
    BadRequest(String),
    /// The request body exceeds the app's payload limit (413).
    #[error("request body is too large")]
    PayloadTooLarge,
    /// The submitted data was rejected (422).
    #[error("{message}")]
    Validation {
        message: String,
        fields: FieldErrors,
    },
    /// A database constraint was violated (409).
    #[error("{0}")]
    Conflict(String),
    /// An unexpected failure (500). The message is logged, not returned.
    #[error("internal error: {0}")]
    Internal(String),
}

impl AdminError {
    pub(crate) fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    /// Submitted fields that were rejected, each with why.
    pub(crate) fn invalid(fields: FieldErrors) -> Self {
        Self::Validation {
            message: "some fields are invalid".to_string(),
            fields,
        }
    }

    /// The HTTP status code for this error.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Validation { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The message sent to the client; internal details are not exposed.
    fn message(&self) -> String {
        match self {
            Self::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        }
    }
}

/// The body of an error response.
#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ErrorBody {
    error: String,
    /// The submitted fields that were rejected, each with why.
    #[serde(skip_serializing_if = "FieldErrors::is_empty")]
    #[cfg_attr(test, ts(as = "Option<FieldErrors>", optional))]
    fields: FieldErrors,
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        if let Self::Internal(msg) = &self {
            tracing::error!(error = %msg, "admin request failed");
        }
        let status = self.status();
        let error = self.message();
        let fields = match self {
            Self::Validation { fields, .. } => fields,
            _ => FieldErrors::new(),
        };
        (status, Json(ErrorBody { error, fields })).into_response()
    }
}

/// Loco serializes model validation failures into `DbErr::Custom` as
/// `{"field": [{"code": "...", "message": "..."}]}`.
#[derive(Deserialize)]
struct LocoValidationMessage {
    code: String,
    message: Option<String>,
}

fn parse_loco_validation(raw: &str) -> Option<FieldErrors> {
    let parsed: BTreeMap<String, Vec<LocoValidationMessage>> = serde_json::from_str(raw).ok()?;
    if parsed.is_empty() {
        return None;
    }
    Some(
        parsed
            .into_iter()
            .map(|(field, msgs)| {
                let msgs = msgs
                    .into_iter()
                    .map(|m| m.message.unwrap_or(m.code))
                    .collect();
                (field, msgs)
            })
            .collect(),
    )
}

fn is_constraint_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    [
        "not null constraint",
        "violates not-null constraint",
        "null value in column",
        "check constraint",
        "violates check constraint",
        "invalid input value for enum",
        "invalid input syntax",
        "value too long",
        "out of range",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// `message` for the client; the database's own wording, which names tables,
/// columns and constraints, goes to the log only.
fn rejected(err: &DbErr, message: &str) -> String {
    tracing::info!(error = %err, "admin request rejected by the database");
    message.to_string()
}

/// The SQLSTATE of an error Postgres returned. `SeaORM` only exposes the
/// driver's error when an sqlx driver feature is on, which the app turns on
/// but this crate cannot require, so it is read from the error's debug form,
/// where sqlx prints it right after the severity:
/// `PgDatabaseError { severity: Error, code: "22003", message: ... }`.
fn postgres_code(err: &DbErr) -> Option<String> {
    const CODE: &str = "code: \"";
    let debug = format!("{err:?}");
    let rest = &debug[debug.find("PgDatabaseError { severity: ")?..];
    let start = rest.find(CODE)? + CODE.len();
    let code = rest.get(start..start + 5)?;
    code.chars()
        .all(|c| c.is_ascii_alphanumeric())
        .then(|| code.to_string())
}

/// Whether Postgres rejected the submitted values: SQLSTATE class 22 (data
/// exception, such as a number out of range or a NUL character in text) or 23
/// (integrity constraint, such as NOT NULL or CHECK). Unlike the message, the
/// code does not depend on the server's language.
fn rejects_values(err: &DbErr) -> bool {
    postgres_code(err).is_some_and(|code| code.starts_with("22") || code.starts_with("23"))
}

impl From<DbErr> for AdminError {
    fn from(err: DbErr) -> Self {
        match err.sql_err() {
            Some(SqlErr::UniqueConstraintViolation(_)) => {
                return Self::Conflict(rejected(
                    &err,
                    "a record with the same value already exists",
                ));
            }
            Some(SqlErr::ForeignKeyConstraintViolation(_)) => {
                return Self::Conflict(rejected(
                    &err,
                    "the change would break a link to another record",
                ));
            }
            _ => {}
        }
        match err {
            // SeaORM never builds `Custom` errors: they carry Loco's validation
            // errors and the messages app hooks reject a write with, both
            // written for the user.
            DbErr::Custom(raw) => match parse_loco_validation(&raw) {
                Some(fields) => Self::Validation {
                    message: "validation failed".to_string(),
                    fields,
                },
                None => Self::Validation {
                    message: raw,
                    fields: FieldErrors::new(),
                },
            },
            DbErr::RecordNotFound(_) | DbErr::RecordNotUpdated => Self::NotFound,
            DbErr::Json(_) | DbErr::Type(_) => {
                Self::BadRequest(rejected(&err, "a value has the wrong type"))
            }
            other => {
                let msg = other.to_string();
                if rejects_values(&other) || is_constraint_message(&msg) {
                    Self::Validation {
                        message: rejected(&other, "the database rejected these values"),
                        fields: FieldErrors::new(),
                    }
                } else {
                    Self::Internal(msg)
                }
            }
        }
    }
}

/// Result alias used throughout the crate.
pub type AdminResult<T> = Result<T, AdminError>;

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::RuntimeErr;

    #[test]
    fn statuses() {
        assert_eq!(AdminError::Unauthenticated.status(), 401);
        assert_eq!(AdminError::Forbidden.status(), 403);
        assert_eq!(AdminError::NotFound.status(), 404);
        assert_eq!(AdminError::bad_request("x").status(), 400);
        assert_eq!(AdminError::PayloadTooLarge.status(), 413);
        assert_eq!(
            AdminError::PayloadTooLarge.to_string(),
            "request body is too large"
        );
        let validation = AdminError::Validation {
            message: "x".into(),
            fields: FieldErrors::new(),
        };
        assert_eq!(validation.status(), 422);
        assert_eq!(AdminError::Conflict("x".into()).status(), 409);
        assert_eq!(AdminError::Internal("x".into()).status(), 500);
    }

    #[test]
    fn display_hides_nothing_but_response_hides_internal() {
        assert_eq!(
            AdminError::Internal("boom".into()).to_string(),
            "internal error: boom"
        );
        assert_eq!(AdminError::NotFound.to_string(), "not found");
        assert_eq!(
            AdminError::Internal("boom".into()).message(),
            "internal server error"
        );
    }

    #[test]
    fn loco_validation_errors_become_field_errors() {
        let raw = r#"{"email":[{"code":"email","message":"invalid email"}],"name":[{"code":"length","message":null}]}"#;
        let err = AdminError::from(DbErr::Custom(raw.to_string()));
        let AdminError::Validation { fields, .. } = err else {
            panic!("expected validation error");
        };
        assert_eq!(fields["email"], vec!["invalid email"]);
        assert_eq!(fields["name"], vec!["length"]);
    }

    #[test]
    fn custom_errors_without_json_are_plain_validation() {
        let err = AdminError::from(DbErr::Custom("nope".into()));
        assert!(
            matches!(err, AdminError::Validation { ref message, ref fields } if message == "nope" && fields.is_empty())
        );
        let err = AdminError::from(DbErr::Custom("{}".into()));
        assert!(matches!(err, AdminError::Validation { ref message, .. } if message == "{}"));
    }

    #[test]
    fn db_error_mapping() {
        assert!(matches!(
            AdminError::from(DbErr::RecordNotFound("x".into())),
            AdminError::NotFound
        ));
        assert!(matches!(
            AdminError::from(DbErr::RecordNotUpdated),
            AdminError::NotFound
        ));
        for err in [
            DbErr::Json("bad secret_col".into()),
            DbErr::Type("bad secret_col".into()),
        ] {
            let err = AdminError::from(err);
            assert!(matches!(err, AdminError::BadRequest(_)));
            assert_eq!(err.message(), "a value has the wrong type");
        }
        let not_null = DbErr::Exec(RuntimeErr::Internal(
            "NOT NULL constraint failed: posts.secret_col".into(),
        ));
        let err = AdminError::from(not_null);
        assert!(matches!(
            err,
            AdminError::Validation { ref message, ref fields }
                if message == "the database rejected these values" && fields.is_empty()
        ));
        let other = DbErr::Conn(RuntimeErr::Internal("connection refused".into()));
        assert!(matches!(AdminError::from(other), AdminError::Internal(_)));
    }

    #[tokio::test]
    async fn response_body_contains_fields() {
        use http_body_util::BodyExt;
        let resp = AdminError::Validation {
            message: "title: is required".into(),
            fields: BTreeMap::from([("title".to_string(), vec!["is required".to_string()])]),
        }
        .into_response();
        assert_eq!(resp.status(), 422);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["fields"]["title"][0], "is required");
        assert_eq!(json["error"], "title: is required");

        let resp = AdminError::Internal("secret".into()).into_response();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(!String::from_utf8_lossy(&body).contains("secret"));
    }
}

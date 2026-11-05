//! Serves the embedded admin UI.

use axum::{
    extract::{Path, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::{api::AdminState, error::AdminError};

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// Placeholder for the mount path, written into `index.html` by the build.
pub(crate) const BASE_PLACEHOLDER: &str = "/__MLA_BASE__";

const NOT_BUILT: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Admin</title>\
                         </head><body><p>The admin UI has not been built. Run <code>npm ci &amp;&amp; \
                         npm run build</code> in <code>frontend/</code>.</p></body></html>";

pub(crate) fn find(path: &str) -> Option<&'static [u8]> {
    ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| *bytes)
}

pub(crate) fn content_type(path: &str) -> &'static str {
    match path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
    {
        Some(ext) => match ext.as_str() {
            "html" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" | "map" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "ico" => "image/x-icon",
            "woff2" => "font/woff2",
            "txt" => "text/plain; charset=utf-8",
            _ => "application/octet-stream",
        },
        None => "application/octet-stream",
    }
}

/// `index.html` with the mount path filled in.
pub(crate) fn index_html(base_path: &str) -> String {
    find("index.html").map_or_else(
        || NOT_BUILT.to_string(),
        |bytes| String::from_utf8_lossy(bytes).replace(BASE_PLACEHOLDER, base_path),
    )
}

pub(crate) async fn asset(Path(path): Path<String>) -> Response {
    let full = format!("assets/{path}");
    match find(&full) {
        Some(bytes) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(content_type(&full)),
                ),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                ),
            ],
            bytes,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

pub(crate) async fn index(State(state): State<AdminState>, method: Method) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return AdminError::NotFound.into_response();
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        index_html(&state.0.base_path),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/a.JS"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("a.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("a.svg"), "image/svg+xml");
        assert_eq!(content_type("a.png"), "image/png");
        assert_eq!(content_type("a.ico"), "image/x-icon");
        assert_eq!(content_type("a.woff2"), "font/woff2");
        assert_eq!(content_type("a.json"), "application/json");
        assert_eq!(content_type("a.txt"), "text/plain; charset=utf-8");
        assert_eq!(content_type("a.bin"), "application/octet-stream");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }

    #[test]
    fn index_uses_the_mount_path() {
        let html = index_html("/backoffice");
        assert!(!html.contains(BASE_PLACEHOLDER));
        if find("index.html").is_some() {
            assert!(html.contains("/backoffice/assets/"));
        }
    }

    #[test]
    fn only_index_references_the_placeholder() {
        for (name, bytes) in ASSETS {
            if *name != "index.html" {
                assert!(
                    !String::from_utf8_lossy(bytes).contains(BASE_PLACEHOLDER),
                    "{name} must not reference the base placeholder"
                );
            }
        }
    }
}

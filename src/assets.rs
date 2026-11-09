//! Serves the embedded admin UI.

use axum::{
    extract::{Path, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

use crate::{api::AdminState, error::AdminError};

/// The built UI (`npm run build` in `frontend/`).
#[derive(Embed)]
#[folder = "frontend/dist/"]
#[allow_missing = true]
struct Dist;

/// Placeholder for the mount path, written into `index.html` by the build.
pub(crate) const BASE_PLACEHOLDER: &str = "/__MLA_BASE__";

const NOT_BUILT: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Muy Loco Admin</title>\
                         </head><body><p>The admin UI has not been built. Run <code>npm ci &amp;&amp; \
                         npm run build</code> in <code>frontend/</code>.</p></body></html>";

/// `index.html` with the mount path filled in.
pub(crate) fn index_html(base_path: &str) -> String {
    Dist::get("index.html").map_or_else(
        || NOT_BUILT.to_string(),
        |file| String::from_utf8_lossy(&file.data).replace(BASE_PLACEHOLDER, base_path),
    )
}

pub(crate) async fn asset(Path(path): Path<String>) -> Response {
    let Some(file) = Dist::get(&format!("assets/{path}")) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let content_type = HeaderValue::from_str(file.metadata.mimetype())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    (
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        file.data,
    )
        .into_response()
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
    fn index_uses_the_mount_path() {
        let html = index_html("/backoffice");
        assert!(!html.contains(BASE_PLACEHOLDER));
        if Dist::get("index.html").is_some() {
            assert!(html.contains("/backoffice/assets/"));
        }
    }

    #[test]
    fn only_index_references_the_placeholder() {
        for name in Dist::iter().filter(|name| name != "index.html") {
            let file = Dist::get(&name).expect("listed file");
            assert!(
                !String::from_utf8_lossy(&file.data).contains(BASE_PLACEHOLDER),
                "{name} must not reference the base placeholder"
            );
        }
    }

    #[test]
    fn assets_have_content_types() {
        for name in Dist::iter() {
            let file = Dist::get(&name).expect("listed file");
            let mime = file.metadata.mimetype();
            match name.rsplit_once('.').map(|(_, ext)| ext) {
                Some("js") => assert!(mime.contains("javascript"), "{name}: {mime}"),
                Some("css") => assert_eq!(mime, "text/css", "{name}"),
                Some("html") => assert_eq!(mime, "text/html", "{name}"),
                _ => {}
            }
        }
    }
}

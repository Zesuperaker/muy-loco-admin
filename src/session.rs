//! The Loco session the admin's login form signs users in with.
//!
//! The UI trades the token from the app's login endpoint for a session cookie
//! (`POST /api/session`) and keeps nothing itself. The session holds the
//! user's [`AdminUser::id`](crate::AdminUser::id) as its user id, the same
//! convention as Loco's `SessionUser`, so the admin and the app share it.

use axum::http::request::Parts;
use loco_rs::{
    app::AppContext,
    auth::{
        cookies::{self, CookieOptions},
        session::Session,
    },
    environment::Environment,
};

/// Whether session cookies can be trusted. Outside `development` and `test`
/// that takes a secret of the app's own: Loco signs with a public default
/// otherwise (and refuses to in `production`), so anyone could forge one.
pub(crate) fn available(ctx: &AppContext) -> bool {
    matches!(
        ctx.environment,
        Environment::Development | Environment::Test
    ) || !cookies::uses_default_secret(&ctx.config)
}

/// The request's session, or `None` when session cookies cannot be trusted.
pub(crate) async fn load(ctx: &AppContext, parts: &Parts) -> loco_rs::Result<Option<Session>> {
    if !available(ctx) {
        return Ok(None);
    }
    Session::from_parts(parts, ctx).await.map(Some)
}

/// The user id the request's session was signed in with.
#[cfg(feature = "jwt")]
pub(crate) async fn user_id(ctx: &AppContext, parts: &Parts) -> loco_rs::Result<Option<String>> {
    Ok(load(ctx, parts)
        .await?
        .and_then(|session| session.user_id()))
}

fn server_side(ctx: &AppContext) -> bool {
    ctx.config
        .auth
        .as_ref()
        .and_then(|auth| auth.session.as_ref())
        .is_some_and(|session| session.store.is_server_side())
}

/// Why the login form could not sign anyone in, checked when the admin is
/// mounted. `middleware` tells whether the admin's stack runs Loco's
/// `auth_session` middleware, which server-side stores write through.
pub(crate) fn check(ctx: &AppContext, middleware: bool) -> Result<(), String> {
    if !available(ctx) {
        return Err(
            "the admin signs users in with a Loco session: set `auth.session.secret` to a long \
             random value (e.g. `openssl rand -base64 64`); outside development and test the \
             admin does not accept sessions signed with Loco's development default"
                .to_string(),
        );
    }
    let name = Session::cookie_name(ctx);
    let lower = name.to_ascii_lowercase();
    if (lower.starts_with("__host-") || lower.starts_with("__secure-"))
        && !CookieOptions::from_config(&ctx.config).secure
    {
        return Err(format!(
            "browsers drop the `{name}` session cookie unless it is `Secure`: serve the app from \
             an `https://` `server.host` or set `auth.session.secure: true`"
        ));
    }
    if server_side(ctx) && !middleware {
        return Err(
            "`auth.session.store` is server-side, but the admin's middleware stack has no \
             `auth_session` middleware to save sessions; add \
             `middleware::auth_session::AuthSession` to `Hooks::middlewares`"
                .to_string(),
        );
    }
    Ok(())
}

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
    config::Config,
    environment::Environment,
};

use crate::auth::development_only_refusal;

/// Why session cookies cannot be trusted, if they cannot. Without a secret
/// of the app's own, Loco signs them with a public default (and refuses to in
/// `production`), so anyone could forge one; that is only accepted in local
/// development, under the same rule as [`AllowAll`](crate::AllowAll).
fn untrusted(config: &Config, environment: &Environment, debug_build: bool) -> Option<String> {
    if !cookies::uses_default_secret(config) {
        return None;
    }
    development_only_refusal(environment, debug_build)
}

/// Whether session cookies can be trusted.
pub(crate) fn available(ctx: &AppContext) -> bool {
    untrusted(&ctx.config, &ctx.environment, cfg!(debug_assertions)).is_none()
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
    if let Some(reason) = untrusted(&ctx.config, &ctx.environment, cfg!(debug_assertions)) {
        return Err(format!(
            "the admin signs users in with a Loco session: set `auth.session.secret` to a long \
             random value (e.g. `openssl rand -base64 64`). Sessions signed with Loco's public \
             development default are only accepted in a debug build in the development or test \
             environment, but {reason}"
        ));
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn config(session: &serde_json::Value) -> Config {
        serde_json::from_value(json!({
            "logger": { "enable": false, "level": "error", "format": "compact" },
            "server": { "port": 5150, "host": "http://localhost" },
            "database": {
                "uri": "sqlite::memory:",
                "enable_logging": false,
                "min_connections": 1,
                "max_connections": 1,
                "connect_timeout": 500,
                "idle_timeout": 500,
                "auto_migrate": false,
                "dangerously_truncate": false,
                "dangerously_recreate": false
            },
            "auth": { "session": session }
        }))
        .expect("valid config")
    }

    #[test]
    fn the_development_secret_is_trusted_only_in_local_development() {
        let default = config(&json!({}));
        let own = config(&json!({ "secret": "an-app-secret-that-is-long-enough-0123456789" }));
        for env in [Environment::Development, Environment::Test] {
            assert_eq!(untrusted(&default, &env, true), None, "{env:?}");
            assert_eq!(
                untrusted(&default, &env, false).as_deref(),
                Some("this is a release build"),
                "{env:?}"
            );
            assert_eq!(untrusted(&own, &env, false), None, "{env:?}");
        }
        for env in [Environment::Production, Environment::Any("staging".into())] {
            assert!(untrusted(&default, &env, true).is_some(), "{env:?}");
            assert!(untrusted(&default, &env, false).is_some(), "{env:?}");
            assert_eq!(untrusted(&own, &env, false), None, "{env:?}");
        }
    }
}

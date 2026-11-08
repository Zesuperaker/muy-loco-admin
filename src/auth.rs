//! Authentication hooks.
//!
//! Every admin API request is authenticated through an [`AdminAuth`]
//! implementation. A user it lets in may use the whole admin: what can be
//! done to each model is set per model with
//! [`ModelAdmin`](crate::ModelAdmin), the same for every admin user.

use async_trait::async_trait;
use axum::http::request::Parts;
use loco_rs::{app::AppContext, environment::Environment};
use serde::Serialize;

use crate::error::AdminError;

/// An authenticated admin user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminUser {
    /// A stable identifier (for example the user's `pid`).
    pub id: String,
    /// The name shown in the UI.
    pub name: String,
}

impl AdminUser {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// Why authentication failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// No valid credentials (401). The UI shows its login screen.
    Unauthenticated,
    /// Valid credentials without admin access (403).
    Forbidden,
    /// Authentication could not be performed (500).
    Internal(String),
}

impl From<AuthError> for AdminError {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::Unauthenticated => Self::Unauthenticated,
            AuthError::Forbidden => Self::Forbidden,
            AuthError::Internal(msg) => Self::Internal(msg),
        }
    }
}

/// A login form that posts JSON credentials to an endpoint and reads a bearer
/// token from the JSON response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormLogin {
    /// URL the credentials are posted to.
    pub endpoint: String,
    /// JSON key of the identifier (for example `email`).
    pub identifier_field: String,
    /// Label of the identifier input.
    pub identifier_label: String,
    /// JSON key of the password.
    pub password_field: String,
    /// JSON key holding the token in the response.
    pub token_field: String,
}

impl Default for FormLogin {
    /// The login endpoint of Loco's `SaaS` starter.
    fn default() -> Self {
        Self {
            endpoint: "/api/auth/login".to_string(),
            identifier_field: "email".to_string(),
            identifier_label: "Email".to_string(),
            password_field: "password".to_string(),
            token_field: "token".to_string(),
        }
    }
}

/// How the UI signs users in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoginMethod {
    /// Show a login form. The UI sends the token it gets back to the admin
    /// once, as `Authorization: Bearer`, and the admin starts a Loco session
    /// with [`AdminUser::id`] as its user id (`Session::user_id`); the
    /// browser keeps only the session cookie. A custom [`AdminAuth`] using
    /// this must accept that token and read the session back.
    Form(FormLogin),
    /// Send the user to the app's own login page.
    Redirect { url: String },
    /// Credentials are handled outside the admin (for example a cookie).
    None,
}

/// Authenticates admin requests.
///
/// Implement this to plug in any authentication scheme. Return an
/// [`AdminUser`] only for users who may use the whole admin.
#[async_trait]
pub trait AdminAuth: Send + Sync + 'static {
    /// Authenticate the request described by `parts`.
    async fn authenticate(&self, ctx: &AppContext, parts: &Parts) -> Result<AdminUser, AuthError>;

    /// How the UI should sign users in.
    fn login(&self) -> LoginMethod {
        LoginMethod::None
    }

    /// Called once when the admin is mounted. An error stops the app from
    /// starting, for auth that must not run where it is mounted.
    fn check(&self, _ctx: &AppContext) -> loco_rs::Result<()> {
        Ok(())
    }
}

/// Lets every request in as an anonymous admin.
///
/// For local development and tests only. It runs in a debug build in the
/// `development` or `test` environment; anywhere else the admin refuses to
/// start. Release builds are refused whatever their environment, so a
/// deployment that forgets `LOCO_ENV`, which Loco then runs as
/// `development`, stays closed. Mounting it logs a warning.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

/// Why something meant only for local development, such as [`AllowAll`] or
/// sessions signed with Loco's public development secret, may not run here,
/// if it may not. It needs a debug build in the `development` or `test`
/// environment. Release builds are refused whatever their environment, so a
/// deployment that forgets `LOCO_ENV`, which Loco then runs as
/// `development`, stays closed.
pub(crate) fn development_only_refusal(
    environment: &Environment,
    debug_build: bool,
) -> Option<String> {
    if !debug_build {
        return Some("this is a release build".to_string());
    }
    if !matches!(environment, Environment::Development | Environment::Test) {
        return Some(format!("the environment is `{environment}`"));
    }
    None
}

#[async_trait]
impl AdminAuth for AllowAll {
    async fn authenticate(&self, ctx: &AppContext, _parts: &Parts) -> Result<AdminUser, AuthError> {
        // Checked when mounting too; this covers auth that wraps `AllowAll`
        // without its `check`.
        if development_only_refusal(&ctx.environment, cfg!(debug_assertions)).is_some() {
            return Err(AuthError::Forbidden);
        }
        Ok(AdminUser::new("anonymous", "Anonymous"))
    }

    fn check(&self, ctx: &AppContext) -> loco_rs::Result<()> {
        if let Some(reason) = development_only_refusal(&ctx.environment, cfg!(debug_assertions)) {
            return Err(loco_rs::Error::string(&format!(
                "AllowAll lets anyone use the admin, so it only runs in a debug build in the \
                 development or test environment, but {reason}; use JwtAuth or your own AdminAuth"
            )));
        }
        tracing::warn!("the admin is open to anyone (AllowAll); use it for local development only");
        Ok(())
    }
}

#[cfg(feature = "jwt")]
pub use jwt::JwtAuth;

#[cfg(feature = "jwt")]
mod jwt {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::http::request::Parts;
    use loco_rs::{
        app::AppContext,
        auth::jwt::JWT,
        controller::extractor::auth::{extract_token, extract_token_from_header},
        model::{Authenticable, ModelError},
    };

    use super::{AdminAuth, AdminUser, AuthError, FormLogin, LoginMethod};
    use crate::session;

    type Check<U> = Arc<dyn Fn(&U) -> bool + Send + Sync>;
    type Name<U> = Arc<dyn Fn(&U) -> String + Send + Sync>;

    /// Authenticates with Loco's JWT auth (`auth.jwt` in the config).
    ///
    /// The admin UI signs in through the app's login endpoint and trades the
    /// token for a Loco session cookie, so the browser never stores the
    /// token. Scripts can send the token instead: it is read from the
    /// locations configured in `auth.jwt.location` and from the
    /// `Authorization: Bearer` header, and wins over a session. Either way the
    /// user is loaded by `pid` with [`Authenticable::find_by_claims_key`] and
    /// must pass the `allow` check on every request.
    ///
    /// The session is the app's own: the admin reads its user id as the
    /// user's `pid`, so the app must sign users in with their `pid` too, and
    /// any way the app signs a user in also opens the admin to admin users.
    ///
    /// ```rust,ignore
    /// JwtAuth::<users::Model>::new(|user| user.is_admin)
    ///     .login_endpoint("/api/auth/login")
    ///     .display_name(|user| user.email.clone())
    /// ```
    pub struct JwtAuth<U> {
        allow: Check<U>,
        name: Option<Name<U>>,
        login: LoginMethod,
    }

    impl<U> JwtAuth<U>
    where
        U: Authenticable + Send + Sync + 'static,
    {
        /// Only users for which `allow` returns `true` can use the admin, and
        /// they can use all of it.
        #[must_use]
        pub fn new(allow: impl Fn(&U) -> bool + Send + Sync + 'static) -> Self {
            Self {
                allow: Arc::new(allow),
                name: None,
                login: LoginMethod::Form(FormLogin::default()),
            }
        }

        /// The name shown for a user (defaults to the token's `pid`).
        #[must_use]
        pub fn display_name(mut self, name: impl Fn(&U) -> String + Send + Sync + 'static) -> Self {
            self.name = Some(Arc::new(name));
            self
        }

        /// Override how the UI signs users in (defaults to
        /// [`FormLogin::default`]).
        #[must_use]
        pub fn login_method(mut self, login: LoginMethod) -> Self {
            self.login = login;
            self
        }

        /// Post the login form to `endpoint`, your app's login route
        /// (defaults to `/api/auth/login`). Other form settings are kept, or
        /// taken from [`FormLogin::default`] if the UI showed no form.
        #[must_use]
        pub fn login_endpoint(mut self, endpoint: impl Into<String>) -> Self {
            let mut form = match self.login {
                LoginMethod::Form(form) => form,
                LoginMethod::Redirect { .. } | LoginMethod::None => FormLogin::default(),
            };
            form.endpoint = endpoint.into();
            self.login = LoginMethod::Form(form);
            self
        }
    }

    #[async_trait]
    impl<U> AdminAuth for JwtAuth<U>
    where
        U: Authenticable + Send + Sync + 'static,
    {
        async fn authenticate(
            &self,
            ctx: &AppContext,
            parts: &Parts,
        ) -> Result<AdminUser, AuthError> {
            let config = ctx.config.get_jwt_config().map_err(|_| {
                AuthError::Internal("JwtAuth requires `auth.jwt` in the Loco config".to_string())
            })?;
            let pid = match extract_token(config, parts)
                .or_else(|_| extract_token_from_header(&parts.headers))
            {
                Ok(token) => {
                    JWT::new(&config.secret)
                        .validate(&token)
                        .map_err(|_| AuthError::Unauthenticated)?
                        .claims
                        .pid
                }
                Err(_) => session::user_id(ctx, parts)
                    .await
                    .map_err(|err| AuthError::Internal(err.to_string()))?
                    .ok_or(AuthError::Unauthenticated)?,
            };
            let user = U::find_by_claims_key(&ctx.db, &pid)
                .await
                .map_err(|err| match err {
                    ModelError::EntityNotFound => AuthError::Unauthenticated,
                    other => AuthError::Internal(other.to_string()),
                })?;
            if !(self.allow)(&user) {
                return Err(AuthError::Forbidden);
            }
            let name = self
                .name
                .as_ref()
                .map_or_else(|| pid.clone(), |name| name(&user));
            Ok(AdminUser::new(pid, name))
        }

        fn login(&self) -> LoginMethod {
            self.login.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all_needs_a_debug_build_in_development_or_test() {
        for env in [Environment::Development, Environment::Test] {
            assert_eq!(development_only_refusal(&env, true), None, "{env:?}");
            assert_eq!(
                development_only_refusal(&env, false).as_deref(),
                Some("this is a release build"),
                "{env:?}"
            );
        }
        for env in [Environment::Production, Environment::Any("staging".into())] {
            let reason = development_only_refusal(&env, true).expect("refused");
            assert!(reason.contains(&env.to_string()), "{reason}");
        }
    }

    #[test]
    fn auth_errors_map_to_admin_errors() {
        assert!(matches!(
            AdminError::from(AuthError::Unauthenticated),
            AdminError::Unauthenticated
        ));
        assert!(matches!(
            AdminError::from(AuthError::Forbidden),
            AdminError::Forbidden
        ));
        assert!(matches!(
            AdminError::from(AuthError::Internal("x".into())),
            AdminError::Internal(_)
        ));
    }

    #[test]
    fn login_methods_serialize_with_a_type_tag() {
        let form = serde_json::to_value(LoginMethod::Form(FormLogin::default())).unwrap();
        assert_eq!(form["type"], "form");
        assert_eq!(form["endpoint"], "/api/auth/login");
        assert_eq!(form["token_field"], "token");
        let redirect = serde_json::to_value(LoginMethod::Redirect {
            url: "/login".into(),
        })
        .unwrap();
        assert_eq!(
            redirect,
            serde_json::json!({"type": "redirect", "url": "/login"})
        );
        assert_eq!(
            serde_json::to_value(LoginMethod::None).unwrap(),
            serde_json::json!({"type": "none"})
        );
    }
}

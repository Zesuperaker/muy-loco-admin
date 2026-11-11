//! Authentication hooks.
//!
//! Every admin API request is authenticated through an [`AdminAuth`]
//! implementation. A user it lets in without a role may use the whole admin;
//! one with a [role](crate::migrations::roles) may only do what the role grants.
//! What can be done to each model at most is set per model with
//! [`ModelAdmin`](crate::ModelAdmin).

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
    /// The user's role (an `admin_roles` id), or `None` for full access.
    pub role: Option<i64>,
}

impl AdminUser {
    /// A user with full access.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            role: None,
        }
    }

    /// Limit the user to role `role` (an `admin_roles` id), or give them
    /// full access with `None`. A role that does not exist lets them do
    /// nothing.
    #[must_use]
    pub fn with_role(mut self, role: Option<i64>) -> Self {
        self.role = role;
        self
    }
}

/// Where an [`AdminAuth`] keeps each user's role; returning it from
/// [`AdminAuth::roles`] turns roles on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRoles {
    /// The users table, as passed to [`migrations::roles`](crate::migrations::roles).
    pub table: String,
    /// The column holding each user's role, `admin_role_id`.
    pub column: String,
    /// Other columns that decide who may use the admin, such as `is_admin`.
    /// Like the role column, only users with full access may change them.
    pub protected: Vec<String>,
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
#[cfg_attr(test, derive(ts_rs::TS))]
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
#[cfg_attr(test, derive(ts_rs::TS))]
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
/// [`AdminUser`] only for users who may use the admin, with their role if
/// they have one ([`AdminUser::with_role`]).
#[async_trait]
pub trait AdminAuth: Send + Sync + 'static {
    /// Authenticate the request described by `parts`.
    async fn authenticate(&self, ctx: &AppContext, parts: &Parts) -> Result<AdminUser, AuthError>;

    /// How the UI should sign users in.
    fn login(&self) -> LoginMethod {
        LoginMethod::None
    }

    /// Where users' roles are kept, if roles are on. Users with full access
    /// can then manage roles in the UI and assign them on the users table.
    fn roles(&self) -> Option<UserRoles> {
        None
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
        config::{JWT as JwtConfig, JWTLocation, JWTLocationConfig},
        controller::extractor::auth::{
            extract_token_from_cookie, extract_token_from_header, extract_token_from_query,
        },
        model::{Authenticable, ModelError},
    };
    use sea_orm::{
        ColumnTrait, EntityName, EntityTrait, IdenStatic, ModelTrait, Value, sea_query::ColumnType,
    };

    use super::{AdminAuth, AdminUser, AuthError, FormLogin, LoginMethod, UserRoles};
    use crate::session;

    type Check<U> = Arc<dyn Fn(&U) -> bool + Send + Sync>;
    type Name<U> = Arc<dyn Fn(&U) -> String + Send + Sync>;
    type ReadRole<U> = Arc<dyn Fn(&U) -> Result<Option<i64>, String> + Send + Sync>;

    struct Roles<U> {
        read: ReadRole<U>,
        columns: UserRoles,
        /// Why the role column cannot hold roles, reported when mounting.
        error: Option<String>,
    }

    /// A role id read from the role column.
    fn role_id(value: &Value) -> Result<Option<i64>, String> {
        match value {
            Value::BigInt(id) => Ok(*id),
            Value::Int(id) => Ok(id.map(i64::from)),
            other => Err(format!("the role column holds {other:?}, not a role id")),
        }
    }

    /// The tokens a request carries, in the order they are tried: the
    /// `Authorization: Bearer` header first, since a client only sends it on
    /// purpose while browsers send cookies on their own, then each location
    /// configured in `auth.jwt.location`.
    fn tokens(config: &JwtConfig, parts: &Parts) -> Vec<String> {
        let configured: Vec<&JWTLocation> = match &config.location {
            Some(JWTLocationConfig::Single(location)) => vec![location],
            Some(JWTLocationConfig::Multiple(locations)) => locations.iter().collect(),
            None => Vec::new(),
        };
        let others = configured
            .into_iter()
            .filter_map(|location| match location {
                JWTLocation::Bearer => None,
                JWTLocation::Query { name } => extract_token_from_query(name, parts).ok(),
                JWTLocation::Cookie { name } => extract_token_from_cookie(name, parts).ok(),
            });
        extract_token_from_header(&parts.headers)
            .ok()
            .into_iter()
            .chain(others)
            .collect()
    }

    /// Authenticates with Loco's JWT auth (`auth.jwt` in the config).
    ///
    /// The admin UI signs in through the app's login endpoint and trades the
    /// token for a Loco session cookie, so the browser never stores the
    /// token. Scripts can send the token instead, as `Authorization: Bearer`
    /// or in the locations configured in `auth.jwt.location`, tried in that
    /// order. The first token that validates is used and wins over a session;
    /// one that does not, such as an expired cookie, is skipped. Either way
    /// the user is loaded by `pid` with [`Authenticable::find_by_claims_key`]
    /// and must pass the `allow` check on every request.
    ///
    /// The session is the app's own: the admin reads its user id as the
    /// user's `pid`, so the app must sign users in with their `pid` too, and
    /// any way the app signs a user in also opens the admin to admin users.
    ///
    /// ```rust,ignore
    /// JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true))
    ///     .login_endpoint("/api/auth/login")
    ///     .display_name(|user| user.email.clone())
    /// ```
    pub struct JwtAuth<U> {
        allow: Check<U>,
        name: Option<Name<U>>,
        login: LoginMethod,
        roles: Option<Roles<U>>,
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
                roles: None,
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

    impl<U> JwtAuth<U>
    where
        U: Authenticable + ModelTrait + Send + Sync + 'static,
    {
        /// Turn on roles. `role` is the column
        /// [`migrations::roles`](crate::migrations::roles) added to the users table;
        /// `access` lists the columns `allow` reads, such as `is_admin`.
        /// Users with full access can then manage roles and assign them; only
        /// they may change `role` and `access`, so a user with a role cannot
        /// hand out access. A role that can edit users can still change their
        /// other columns, such as a full admin's email.
        ///
        /// ```rust,ignore
        /// JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true))
        ///     .roles(users::Column::AdminRoleId, [users::Column::IsAdmin])
        /// ```
        #[must_use]
        pub fn roles(
            mut self,
            role: <U::Entity as EntityTrait>::Column,
            access: impl IntoIterator<Item = <U::Entity as EntityTrait>::Column>,
        ) -> Self {
            let def = role.def();
            let error = (!matches!(def.get_column_type(), ColumnType::BigInteger)
                || !def.is_null())
            .then(|| {
                format!(
                    "the role column `{}` must be the nullable bigint column that \
                     `muy_loco_admin::migrations::roles` adds; run `cargo loco db entities` after \
                     migrating",
                    role.as_str()
                )
            });
            self.roles = Some(Roles {
                read: Arc::new(move |user: &U| role_id(&user.get(role))),
                columns: UserRoles {
                    table: U::Entity::default().table_name().to_string(),
                    column: role.as_str().to_string(),
                    protected: access
                        .into_iter()
                        .map(|column| column.as_str().to_string())
                        .collect(),
                },
                error,
            });
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
            let jwt = JWT::new(&config.secret);
            // A token that does not validate is skipped rather than rejected,
            // so a stale cookie cannot lock out a valid token or session.
            let valid = tokens(config, parts)
                .iter()
                .find_map(|token| jwt.validate(token).ok());
            let pid = match valid {
                Some(data) => data.claims.pid,
                None => session::user_id(ctx, parts)
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
            let role = match &self.roles {
                Some(roles) => (roles.read)(&user).map_err(AuthError::Internal)?,
                None => None,
            };
            let name = self
                .name
                .as_ref()
                .map_or_else(|| pid.clone(), |name| name(&user));
            Ok(AdminUser::new(pid, name).with_role(role))
        }

        fn login(&self) -> LoginMethod {
            self.login.clone()
        }

        fn roles(&self) -> Option<UserRoles> {
            self.roles.as_ref().map(|roles| roles.columns.clone())
        }

        fn check(&self, _ctx: &AppContext) -> loco_rs::Result<()> {
            match self.roles.as_ref().and_then(|roles| roles.error.as_ref()) {
                Some(error) => Err(loco_rs::Error::string(error)),
                None => Ok(()),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn role_ids_are_read_from_integer_columns() {
            assert_eq!(role_id(&Value::BigInt(Some(4))), Ok(Some(4)));
            assert_eq!(role_id(&Value::BigInt(None)), Ok(None));
            assert_eq!(role_id(&Value::Int(Some(2))), Ok(Some(2)));
            assert!(role_id(&Value::String(Some("x".into()))).is_err());
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

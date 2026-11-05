//! Authentication and authorization hooks.
//!
//! Every admin API request is authenticated through an [`AdminAuth`]
//! implementation. The returned [`AdminUser`] then decides, per model and
//! operation, what the user may do.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use axum::http::request::Parts;
use loco_rs::{app::AppContext, environment::Environment};
use serde::Serialize;

use crate::error::AdminError;

/// An operation on a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation<'a> {
    /// List, view and look up records.
    View,
    Create,
    Update,
    Delete,
    /// Download records as CSV.
    Export,
    /// Run the named custom action.
    Action(&'a str),
}

/// An authorization request: may the user perform `operation` on `model`?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access<'a> {
    /// The admin model name (the table name unless overridden).
    pub model: &'a str,
    pub operation: Operation<'a>,
}

type Policy = Arc<dyn Fn(&Access<'_>) -> bool + Send + Sync>;

/// An authenticated admin user.
#[derive(Clone)]
pub struct AdminUser {
    /// A stable identifier (for example the user's `pid`).
    pub id: String,
    /// The name shown in the UI.
    pub name: String,
    policy: Option<Policy>,
}

impl AdminUser {
    /// A user allowed to perform every operation.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            policy: None,
        }
    }

    /// Restrict the user to operations for which `policy` returns `true`.
    #[must_use]
    pub fn with_policy(
        mut self,
        policy: impl Fn(&Access<'_>) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.policy = Some(Arc::new(policy));
        self
    }

    /// Whether the user may perform `access`.
    #[must_use]
    pub fn can(&self, access: &Access<'_>) -> bool {
        self.policy.as_ref().is_none_or(|policy| policy(access))
    }
}

impl fmt::Debug for AdminUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdminUser")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("policy", &self.policy.is_some())
            .finish()
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
    /// Show a login form; the token is sent as `Authorization: Bearer`.
    Form(FormLogin),
    /// Send the user to the app's own login page.
    Redirect { url: String },
    /// Credentials are handled outside the admin (for example a cookie).
    None,
}

/// Authenticates admin requests.
///
/// Implement this to plug in any authentication scheme. The returned
/// [`AdminUser`] carries the per-operation policy.
#[async_trait]
pub trait AdminAuth: Send + Sync + 'static {
    /// Authenticate the request described by `parts`.
    async fn authenticate(&self, ctx: &AppContext, parts: &Parts) -> Result<AdminUser, AuthError>;

    /// How the UI should sign users in.
    fn login(&self) -> LoginMethod {
        LoginMethod::None
    }
}

/// Lets every request in as an anonymous admin.
///
/// Meant for local development and tests. Requests are refused (403) when the
/// app runs in the `production` environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

#[async_trait]
impl AdminAuth for AllowAll {
    async fn authenticate(&self, ctx: &AppContext, _parts: &Parts) -> Result<AdminUser, AuthError> {
        if ctx.environment == Environment::Production {
            return Err(AuthError::Forbidden);
        }
        Ok(AdminUser::new("anonymous", "Anonymous"))
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

    use super::{Access, AdminAuth, AdminUser, AuthError, FormLogin, LoginMethod};

    type Check<U> = Arc<dyn Fn(&U) -> bool + Send + Sync>;
    type Name<U> = Arc<dyn Fn(&U) -> String + Send + Sync>;
    type Permissions<U> = Arc<dyn Fn(&U, &Access<'_>) -> bool + Send + Sync>;

    /// Authenticates with Loco's JWT auth (`auth.jwt` in the config).
    ///
    /// The token is read from the locations configured in `auth.jwt.location`
    /// and from the `Authorization: Bearer` header. The user is loaded with
    /// [`Authenticable::find_by_claims_key`] and must pass the `allow` check.
    ///
    /// ```rust,ignore
    /// JwtAuth::<users::Model>::new(|user| user.is_admin)
    ///     .login_endpoint("/api/auth/login")
    ///     .display_name(|user| user.email.clone())
    ///     .permissions(|user, access| user.is_superuser || access.operation == Operation::View)
    /// ```
    pub struct JwtAuth<U> {
        allow: Check<U>,
        name: Option<Name<U>>,
        permissions: Option<Permissions<U>>,
        login: LoginMethod,
    }

    impl<U> JwtAuth<U>
    where
        U: Authenticable + Send + Sync + 'static,
    {
        /// Only users for which `allow` returns `true` can use the admin.
        #[must_use]
        pub fn new(allow: impl Fn(&U) -> bool + Send + Sync + 'static) -> Self {
            Self {
                allow: Arc::new(allow),
                name: None,
                permissions: None,
                login: LoginMethod::Form(FormLogin::default()),
            }
        }

        /// The name shown for a user (defaults to the token's `pid`).
        #[must_use]
        pub fn display_name(mut self, name: impl Fn(&U) -> String + Send + Sync + 'static) -> Self {
            self.name = Some(Arc::new(name));
            self
        }

        /// Restrict what each user may do. Without this, allowed users may do
        /// everything.
        #[must_use]
        pub fn permissions(
            mut self,
            permissions: impl Fn(&U, &Access<'_>) -> bool + Send + Sync + 'static,
        ) -> Self {
            self.permissions = Some(Arc::new(permissions));
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
            let token = extract_token(config, parts)
                .or_else(|_| extract_token_from_header(&parts.headers))
                .map_err(|_| AuthError::Unauthenticated)?;
            let claims = JWT::new(&config.secret)
                .validate(&token)
                .map_err(|_| AuthError::Unauthenticated)?
                .claims;
            let user =
                U::find_by_claims_key(&ctx.db, &claims.pid)
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
                .map_or_else(|| claims.pid.clone(), |name| name(&user));
            let admin = AdminUser::new(claims.pid, name);
            Ok(match &self.permissions {
                Some(permissions) => {
                    let permissions = Arc::clone(permissions);
                    let user = Arc::new(user);
                    admin.with_policy(move |access| permissions(&user, access))
                }
                None => admin,
            })
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
    fn users_without_policy_can_do_everything() {
        let user = AdminUser::new("1", "Ann");
        assert!(user.can(&Access {
            model: "posts",
            operation: Operation::Delete
        }));
        assert_eq!(
            format!("{user:?}"),
            r#"AdminUser { id: "1", name: "Ann", policy: false }"#
        );
    }

    #[test]
    fn policies_restrict_operations() {
        let user = AdminUser::new("1", "Ann")
            .with_policy(|a| a.operation == Operation::View || a.model == "notes");
        assert!(user.can(&Access {
            model: "posts",
            operation: Operation::View
        }));
        assert!(!user.can(&Access {
            model: "posts",
            operation: Operation::Action("publish")
        }));
        assert!(user.can(&Access {
            model: "notes",
            operation: Operation::Delete
        }));
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

//! The [`Admin`] builder and its Loco initializer.

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{
    Router,
    http::{HeaderValue, header},
    middleware::map_response,
    response::{Redirect, Response},
    routing::get,
};
use loco_rs::{
    Error,
    app::{AppContext, Hooks, Initializer},
    controller::middleware::{MiddlewareLayer, default_middleware_stack},
};
use sea_orm::{ActiveModelBehavior, EntityTrait, IntoActiveModel};
use serde::Deserialize;

use crate::{
    api::{self, AdminState, Inner, Model},
    assets,
    auth::{AdminAuth, LoginMethod, UserRoles},
    meta::{ModelMeta, RelatedMeta},
    model::ModelAdmin,
    resource::{ModelResource, Resource},
    roles::{self, ROLES_TABLE},
    session,
};

const DEFAULT_PATH: &str = "/admin";
const DEFAULT_EXPORT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Loco middlewares the admin does not apply. `fallback` and `static` install
/// routes instead of wrapping requests and would shadow the admin's own.
/// `cors` would let origins the app trusts call the admin API with the
/// signed-in user's session cookie; the admin UI never needs it, since it is
/// always served from the same origin as its API.
const SKIPPED_MIDDLEWARES: &[&str] = &["cors", "fallback", "static"];
const DEFAULT_TITLE: &str = "Admin";
const DEFAULT_SITE_URL: &str = "/";
/// Key of the admin settings under `initializers:` in the Loco config.
pub const CONFIG_KEY: &str = "admin";

/// The admin dashboard. Register models, then add it to the app's
/// initializers.
///
/// ```rust,ignore
/// async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
///     Ok(vec![Box::new(
///         Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true)))
///             .register(ModelAdmin::<users::Entity>::new().can_create(false))
///             .model::<articles::Entity>()
///             .middlewares_from::<Self>(),
///     )])
/// }
/// ```
///
/// Admin requests go through the app's Loco middleware stack (except the
/// `cors`, `fallback` and `static` middlewares). Pass the app's hooks with
/// [`Admin::middlewares_from`] so middleware added or removed in
/// `Hooks::middlewares` applies too; otherwise the stack configured under
/// `server.middlewares` is used.
///
/// The mount path, title and site URL can be overridden in the Loco config:
///
/// ```yaml
/// initializers:
///   admin:
///     enabled: true
///     path: /admin
///     title: My App Admin
///     site_url: /
/// ```
pub struct Admin {
    title: String,
    path: String,
    site_url: String,
    auth: Arc<dyn AdminAuth>,
    resources: Vec<Arc<dyn Resource>>,
    errors: Vec<String>,
    middlewares: MiddlewareStack,
    export_idle_timeout: Duration,
}

/// Builds the Loco middleware stack, like `Hooks::middlewares`.
type MiddlewareStack = fn(&AppContext) -> Vec<Box<dyn MiddlewareLayer>>;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    enabled: Option<bool>,
    path: Option<String>,
    title: Option<String>,
    site_url: Option<String>,
}

/// The path is written unescaped into `index.html` and the trailing-slash
/// redirect, so only plain segments of unreserved URL characters are allowed:
/// no markup, quotes, empty (`//host`) or dot segments.
fn validate_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim().trim_end_matches('/');
    let Some(rest) = trimmed.strip_prefix('/').filter(|rest| !rest.is_empty()) else {
        return Err(format!(
            "admin path `{path}` must start with `/` and not be the root"
        ));
    };
    let plain = |segment: &str| {
        !matches!(segment, "" | "." | "..")
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
    };
    if !rest.split('/').all(plain) {
        return Err(format!(
            "admin path `{path}` contains invalid characters: use letters, digits, `-`, `_`, \
             `.` and `~` between single slashes"
        ));
    }
    Ok(trimmed.to_string())
}

/// The "Visit site" link's target: empty (no link), a path on this host or an
/// `http(s)` URL, so a configured value cannot become a `javascript:` link or
/// a protocol-relative one (`//host`). Backslashes are refused too: browsers
/// read them as `/` in web URLs, so `/\host` is protocol-relative as well.
fn validate_site_url(url: &str) -> Result<Option<String>, String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(None);
    }
    let local = url.starts_with('/') && !url.starts_with("//");
    let web = ["http://", "https://"].iter().any(|scheme| {
        url.get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
    });
    let unsafe_char = |c: char| c.is_control() || c.is_whitespace() || c == '\\';
    if (local || web) && !url.chars().any(unsafe_char) {
        Ok(Some(url.to_string()))
    } else {
        Err(format!(
            "admin site_url `{url}` must be a path starting with `/` or an http(s) URL"
        ))
    }
}

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
                   connect-src 'self'; font-src 'self' data:; object-src 'none'; base-uri 'self'; \
                   form-action 'self'; frame-ancestors 'none'";

/// Runs outside the app's middleware: the admin UI needs its own CSP (Loco's
/// presets either require HTTPS for scripts or allow inline styles), and
/// `Clear-Site-Data` on every response would sign the user out.
async fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.remove("clear-site-data");
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

impl Admin {
    /// An admin protected by `auth`, mounted at `/admin`.
    #[must_use]
    pub fn new(auth: impl AdminAuth) -> Self {
        Self {
            title: DEFAULT_TITLE.to_string(),
            path: DEFAULT_PATH.to_string(),
            site_url: DEFAULT_SITE_URL.to_string(),
            auth: Arc::new(auth),
            resources: Vec::new(),
            errors: Vec::new(),
            middlewares: default_middleware_stack,
            export_idle_timeout: DEFAULT_EXPORT_IDLE_TIMEOUT,
        }
    }

    /// Wrap admin requests in the middleware stack of the app `H`
    /// (`H::middlewares`), the same stack Loco applies to the app's routes.
    /// Call it with `Self` inside `Hooks::initializers`.
    #[must_use]
    pub fn middlewares_from<H: Hooks>(mut self) -> Self {
        self.middlewares = H::middlewares;
        self
    }

    /// Title shown in the UI.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Where the UI's "Visit site" link points: a path on this host or an
    /// `http(s)` URL (defaults to `/`). An empty string hides the link.
    #[must_use]
    pub fn site_url(mut self, url: impl Into<String>) -> Self {
        self.site_url = url.into();
        self
    }

    /// How long a CSV export may wait for the client to take the next chunk
    /// (or for the database to produce it) before it is stopped and its
    /// transaction rolled back. Defaults to 60 seconds.
    #[must_use]
    pub fn export_idle_timeout(mut self, timeout: Duration) -> Self {
        self.export_idle_timeout = timeout;
        self
    }

    /// Mount path (defaults to `/admin`).
    #[must_use]
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// Add a configured model.
    #[must_use]
    pub fn register<E>(mut self, model: ModelAdmin<E>) -> Self
    where
        E: EntityTrait + Sync,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync,
        E::ActiveModel: ActiveModelBehavior + Send + Sync,
    {
        match ModelResource::new(model) {
            Ok(resource) => self.resources.push(Arc::new(resource)),
            Err(err) => self.errors.push(err),
        }
        self
    }

    /// Add a model with the default configuration.
    #[must_use]
    pub fn model<E>(self) -> Self
    where
        E: EntityTrait + Sync,
        E::Model: IntoActiveModel<E::ActiveModel> + Sync,
        E::ActiveModel: ActiveModelBehavior + Send + Sync,
    {
        self.register(ModelAdmin::<E>::new())
    }

    /// Build the admin router, to be nested at [`Admin::mount_path`].
    ///
    /// The initializer does this for you; use it to mount the admin by hand.
    /// Like the initializer, it checks the database, so it fails when roles
    /// are on and their migration has not run.
    pub async fn router(&self, ctx: &AppContext) -> loco_rs::Result<Router> {
        let router = self.build(ctx, &Settings::default())?;
        self.check_database(ctx).await?;
        Ok(router)
    }

    /// Check that the database has what the admin needs: with roles on, the
    /// roles table and the users' role column.
    async fn check_database(&self, ctx: &AppContext) -> loco_rs::Result<()> {
        match self.auth.roles() {
            Some(users) => roles::check_tables(&ctx.db, &users)
                .await
                .map_err(|e| Error::string(&e)),
            None => Ok(()),
        }
    }

    /// The validated mount path.
    pub fn mount_path(&self) -> loco_rs::Result<String> {
        validate_path(&self.path).map_err(|e| Error::string(&e))
    }

    /// The registered models' metadata, with relations resolved against
    /// each other and the users' role column marked.
    fn metas(&self, user_roles: Option<&UserRoles>) -> loco_rs::Result<Vec<ModelMeta>> {
        let mut tables: HashMap<String, String> = HashMap::new();
        let mut names = std::collections::HashSet::new();
        for resource in &self.resources {
            let meta = resource.meta();
            if user_roles.is_some() && meta.table == ROLES_TABLE {
                // Roles would be editable by anyone allowed to edit the
                // model, including users limited by a role.
                return Err(Error::string(&format!(
                    "model `{}`: the `{ROLES_TABLE}` table is managed on the admin's Roles \
                     page and cannot be registered",
                    meta.name
                )));
            }
            if !names.insert(meta.name.clone()) {
                return Err(Error::string(&format!(
                    "model `{}` is registered more than once",
                    meta.name
                )));
            }
            tables
                .entry(meta.table.clone())
                .or_insert_with(|| meta.name.clone());
        }

        let mut metas: Vec<_> = self.resources.iter().map(|r| r.meta().clone()).collect();
        for meta in &mut metas {
            let users = user_roles.filter(|r| r.table == meta.table);
            for field in &mut meta.fields {
                if users.is_some_and(|r| r.column == field.name) {
                    // Picked from the roles, not a linked model.
                    field.role = true;
                    field.relation = None;
                }
                if let Some(rel) = &mut field.relation {
                    rel.model = tables.get(&rel.table).cloned();
                }
            }
        }
        let mut related: Vec<(String, RelatedMeta)> = Vec::new();
        for meta in &metas {
            for field in &meta.fields {
                if let Some(target) = field.relation.as_ref().and_then(|r| r.model.clone()) {
                    related.push((
                        target,
                        RelatedMeta {
                            model: meta.name.clone(),
                            field: field.name.clone(),
                            label: meta.label.clone(),
                        },
                    ));
                }
            }
        }
        for (target, rel) in related {
            if let Some(meta) = metas.iter_mut().find(|m| m.name == target) {
                meta.related.push(rel);
            }
        }

        Ok(metas)
    }

    /// Build the router, with `settings` from the Loco config taking
    /// precedence over the builder's values.
    fn build(&self, ctx: &AppContext, settings: &Settings) -> loco_rs::Result<Router> {
        if !self.errors.is_empty() {
            return Err(Error::string(&self.errors.join("; ")));
        }
        self.auth.check(ctx)?;
        let user_roles = self.auth.roles();
        let path = settings.path.as_deref().unwrap_or(&self.path);
        let base_path = validate_path(path).map_err(|e| Error::string(&e))?;
        let title = settings.title.as_deref().unwrap_or(&self.title);
        let site_url = validate_site_url(settings.site_url.as_deref().unwrap_or(&self.site_url))
            .map_err(|e| Error::string(&e))?;

        let metas = self.metas(user_roles.as_ref())?;

        let models = self
            .resources
            .iter()
            .zip(metas)
            .map(|(resource, meta)| Model {
                resource: Arc::clone(resource),
                meta,
            })
            .collect();
        let state = AdminState(Arc::new(Inner {
            title: title.to_string(),
            site_url,
            export_idle_timeout: self.export_idle_timeout,
            base_path,
            auth: Arc::clone(&self.auth),
            roles: user_roles,
            models,
            ctx: ctx.clone(),
        }));

        let mut router: Router<AppContext> = Router::new()
            .nest("/api", api::router())
            .route("/assets/{*path}", get(assets::asset))
            .route("/", get(assets::index))
            .fallback(assets::index)
            .with_state(state);
        let mut sessions_saved = false;
        for middleware in (self.middlewares)(ctx) {
            if middleware.is_enabled() && !SKIPPED_MIDDLEWARES.contains(&middleware.name()) {
                router = middleware.apply(router)?;
                sessions_saved |= middleware.name() == "auth_session";
                tracing::debug!(name = middleware.name(), "admin +middleware");
            }
        }
        if matches!(self.auth.login(), LoginMethod::Form(_)) {
            session::check(ctx, sessions_saved).map_err(|e| Error::string(&e))?;
        }
        Ok(router
            .layer(map_response(security_headers))
            .with_state(ctx.clone()))
    }

    fn settings(ctx: &AppContext) -> loco_rs::Result<Settings> {
        let Some(raw) = ctx
            .config
            .initializers
            .as_ref()
            .and_then(|i| i.get(CONFIG_KEY))
        else {
            return Ok(Settings::default());
        };
        serde_json::from_value(raw.clone())
            .map_err(|e| Error::string(&format!("invalid `initializers.{CONFIG_KEY}` config: {e}")))
    }
}

#[async_trait]
impl Initializer for Admin {
    fn name(&self) -> String {
        CONFIG_KEY.to_string()
    }

    async fn after_routes(&self, router: Router, ctx: &AppContext) -> loco_rs::Result<Router> {
        let settings = Self::settings(ctx)?;
        if settings.enabled == Some(false) {
            return Ok(router);
        }
        let admin = self.build(ctx, &settings)?;
        self.check_database(ctx).await?;
        let path = settings.path.as_deref().unwrap_or(&self.path);
        let mount = validate_path(path).map_err(|e| Error::string(&e))?;
        tracing::info!(path = %mount, models = self.resources.len(), "admin mounted");
        let canonical = mount.clone();
        Ok(router.nest(&mount, admin).route(
            &format!("{mount}/"),
            get(move || async move { Redirect::permanent(&canonical) }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_validated() {
        assert_eq!(validate_path("/admin").unwrap(), "/admin");
        assert_eq!(validate_path("/admin/").unwrap(), "/admin");
        assert_eq!(validate_path(" /ops/admin ").unwrap(), "/ops/admin");
        assert!(validate_path("/").is_err());
        assert!(validate_path("admin").is_err());
        assert!(validate_path("").is_err());
        assert!(validate_path("/{x}").is_err());
        assert!(validate_path("/a b").is_err());
        assert_eq!(
            validate_path("/ops/back-office_v2.~x").unwrap(),
            "/ops/back-office_v2.~x"
        );
        for path in [
            "//evil.example",
            "/a//b",
            "/a/../b",
            "/.",
            "/admin\"><script>",
            "/admin'",
            "/admin<",
            "/admin%2F",
            "/admin\\x",
            "/ädmin",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn site_urls_are_validated() {
        for url in ["", "  "] {
            assert_eq!(validate_site_url(url).unwrap(), None, "{url:?}");
        }
        for url in [
            "/",
            " /home ",
            "/a/b?c=d#e",
            "https://example.com",
            "HTTP://x.test/",
        ] {
            assert_eq!(validate_site_url(url).unwrap().as_deref(), Some(url.trim()));
        }
        for url in [
            "javascript:alert(1)",
            "data:text/html,x",
            "//evil.example",
            "/\\evil.example",
            "/\\/evil.example",
            "/a\\b",
            "https://x.test\\@evil.example",
            "example.com",
            "https",
            "/a b",
            "/a\nb",
            "/a\u{7}",
        ] {
            assert!(validate_site_url(url).is_err(), "{url}");
        }
    }
}

//! The [`Admin`] builder and its Loco initializer.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use axum::{
    Router,
    http::{HeaderValue, header},
    middleware::map_response,
    response::Response,
    routing::get,
};
use loco_rs::{
    Error,
    app::{AppContext, Initializer},
};
use sea_orm::{ActiveModelBehavior, EntityTrait, IntoActiveModel};
use serde::Deserialize;

use crate::{
    api::{self, AdminState, Inner, Model},
    assets,
    auth::AdminAuth,
    meta::RelatedMeta,
    model::ModelAdmin,
    resource::{ModelResource, Resource},
};

const DEFAULT_PATH: &str = "/admin";
const DEFAULT_TITLE: &str = "Admin";
/// Key of the admin settings under `initializers:` in the Loco config.
pub const CONFIG_KEY: &str = "admin";

/// The admin dashboard. Register models, then add it to the app's
/// initializers.
///
/// ```rust,ignore
/// async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
///     Ok(vec![Box::new(
///         Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin))
///             .register(ModelAdmin::<users::Entity>::new().can_create(false))
///             .model::<posts::Entity>(),
///     )])
/// }
/// ```
///
/// The mount path and title can be overridden in the Loco config:
///
/// ```yaml
/// initializers:
///   admin:
///     enabled: true
///     path: /admin
///     title: My App Admin
/// ```
pub struct Admin {
    title: String,
    path: String,
    auth: Arc<dyn AdminAuth>,
    resources: Vec<Arc<dyn Resource>>,
    errors: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    enabled: Option<bool>,
    path: Option<String>,
    title: Option<String>,
}

fn validate_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim().trim_end_matches('/');
    if !trimmed.starts_with('/') || trimmed.len() < 2 {
        return Err(format!(
            "admin path `{path}` must start with `/` and not be the root"
        ));
    }
    if trimmed.contains(['{', '}', '*', '?', '#', ' ']) {
        return Err(format!("admin path `{path}` contains invalid characters"));
    }
    Ok(trimmed.to_string())
}

async fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
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
            auth: Arc::new(auth),
            resources: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Title shown in the UI.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
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
    pub fn router(&self, ctx: &AppContext) -> loco_rs::Result<Router> {
        self.build(ctx, &self.path, &self.title)
    }

    /// The validated mount path.
    pub fn mount_path(&self) -> loco_rs::Result<String> {
        validate_path(&self.path).map_err(|e| Error::string(&e))
    }

    fn build(&self, ctx: &AppContext, path: &str, title: &str) -> loco_rs::Result<Router> {
        if !self.errors.is_empty() {
            return Err(Error::string(&self.errors.join("; ")));
        }
        let base_path = validate_path(path).map_err(|e| Error::string(&e))?;

        let mut tables: HashMap<String, String> = HashMap::new();
        let mut names = std::collections::HashSet::new();
        for resource in &self.resources {
            let meta = resource.meta();
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
            for field in &mut meta.fields {
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
            base_path,
            auth: Arc::clone(&self.auth),
            models,
            ctx: ctx.clone(),
        }));

        Ok(Router::new()
            .nest("/api", api::router())
            .route("/assets/{*path}", get(assets::asset))
            .route("/", get(assets::index))
            .fallback(assets::index)
            .layer(map_response(security_headers))
            .with_state(state))
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
        let path = settings.path.as_deref().unwrap_or(&self.path);
        let title = settings.title.as_deref().unwrap_or(&self.title);
        let admin = self.build(ctx, path, title)?;
        let mount = validate_path(path).map_err(|e| Error::string(&e))?;
        tracing::info!(path = %mount, models = self.resources.len(), "admin mounted");
        Ok(router.nest(&mount, admin))
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
    }
}

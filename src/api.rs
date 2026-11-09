//! JSON API handlers.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{FromRequestParts, Path, Query, Request, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header, request::Parts},
    middleware::{Next, from_fn},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use loco_rs::{app::AppContext, auth::session::Session};
use serde::Deserialize;
use serde_json::{Map, Value as JsonValue, json};

use crate::{
    auth::{AdminAuth, AdminUser, LoginMethod, UserRoles},
    error::{AdminError, AdminResult},
    meta::{Capabilities, ModelMeta},
    query::{ListParams, MAX_PER_PAGE},
    resource::{CsvStream, Record, Resource},
    roles::{self, Access, Role, RoleInput},
    session,
};

/// Header every mutating request must carry. Browsers only let another
/// origin set it after a CORS preflight the admin never approves, so other
/// sites cannot send writes with the user's session cookie.
pub const CSRF_HEADER: &str = "x-requested-with";
/// Expected value of [`CSRF_HEADER`].
pub const CSRF_VALUE: &str = "muy-loco-admin";

const LOOKUP_LIMIT: u64 = 20;
const MAX_LOOKUP_LIMIT: u64 = 100;

pub(crate) struct Model {
    pub resource: Arc<dyn Resource>,
    /// Metadata with relations resolved against the other registered models.
    pub meta: ModelMeta,
}

pub(crate) struct Inner {
    /// Target of the UI's "Visit site" link; `None` hides it.
    pub site_url: Option<String>,
    pub export_idle_timeout: Duration,
    pub base_path: String,
    pub auth: Arc<dyn AdminAuth>,
    /// Where users' roles are kept, when roles are on.
    pub roles: Option<UserRoles>,
    pub models: Vec<Model>,
    pub ctx: AppContext,
}

#[derive(Clone)]
pub(crate) struct AdminState(pub Arc<Inner>);

/// An operation on a model.
#[derive(Clone, Copy)]
enum Operation<'a> {
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

impl AdminState {
    fn model(&self, name: &str) -> Option<&Model> {
        self.0.models.iter().find(|m| m.meta.name == name)
    }

    /// Resolve `name`, checking that its [`ModelAdmin`](crate::ModelAdmin)
    /// allows `operation`, and so does the user's role if they have one.
    fn authorize(
        &self,
        access: &Access,
        name: &str,
        operation: Operation<'_>,
    ) -> AdminResult<&Model> {
        let model = self.model(name).ok_or(AdminError::NotFound)?;
        let caps = access.capabilities(&model.meta);
        let capable = match operation {
            Operation::View => caps.view,
            Operation::Create => caps.create,
            Operation::Update => caps.update,
            Operation::Delete => caps.delete,
            Operation::Export => caps.export,
            Operation::Action(action) => {
                if !model.meta.actions.iter().any(|a| a.name == action) {
                    return Err(AdminError::NotFound);
                }
                access.can_run(&model.meta, action)
            }
        };
        if !capable {
            return Err(AdminError::Forbidden);
        }
        Ok(model)
    }

    /// Fields of `model` the user may not set although the model allows it:
    /// for a user with a role, the users' role column and the other columns
    /// that decide who may use the admin.
    fn locked(&self, model: &Model, access: &Access) -> Vec<String> {
        match &self.0.roles {
            Some(roles) if !access.is_full() && roles.table == model.meta.table => {
                std::iter::once(&roles.column)
                    .chain(&roles.protected)
                    .cloned()
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// `model` as the user sees it, or `None` if they may not view it: with
    /// a role, its capabilities and actions are narrowed, locked fields are
    /// read-only, and links to models the role cannot view are dropped, so
    /// foreign keys to them are edited as plain values.
    fn visible<'a>(&self, model: &'a Model, access: &Access) -> Option<Cow<'a, ModelMeta>> {
        if access.is_full() {
            return Some(Cow::Borrowed(&model.meta));
        }
        let caps = access.capabilities(&model.meta);
        if !caps.view {
            return None;
        }
        let viewable = |name: &str| self.model(name).is_some_and(|m| access.can_view(&m.meta));
        let locked = self.locked(model, access);
        let mut meta = model.meta.clone();
        for field in &mut meta.fields {
            if locked.contains(&field.name) {
                field.readonly = true;
            }
            if let Some(rel) = &mut field.relation
                && rel.model.as_deref().is_some_and(|target| !viewable(target))
            {
                rel.model = None;
            }
        }
        meta.related.retain(|rel| viewable(&rel.model));
        meta.actions
            .retain(|action| access.can_run(&model.meta, &action.name));
        let editable = meta.fields.iter().any(|f| !f.readonly && !f.primary_key);
        meta.capabilities = Capabilities {
            update: caps.update && editable,
            ..caps
        };
        Some(Cow::Owned(meta))
    }

    /// The users' role settings, if the user may manage roles: roles are on
    /// and the user has full access.
    fn role_admin(&self, access: &Access) -> AdminResult<&UserRoles> {
        let roles = self.0.roles.as_ref().ok_or(AdminError::NotFound)?;
        if !access.is_full() {
            return Err(AdminError::Forbidden);
        }
        Ok(roles)
    }

    fn metas(&self) -> Vec<&ModelMeta> {
        self.0.models.iter().map(|m| &m.meta).collect()
    }
}

/// Refuse API requests that a page on another origin made, and writes that
/// lack [`CSRF_HEADER`], before any handler runs.
///
/// Browsers send `Sec-Fetch-Site` with every request; the admin's own pages
/// send `same-origin`, and `none` is a URL the user typed. Requests without it
/// (scripts, older browsers) pass, and still need the header to write.
fn check_origin(method: &Method, headers: &HeaderMap) -> AdminResult<()> {
    if let Some(site) = headers.get("sec-fetch-site")
        && !matches!(site.as_bytes(), b"same-origin" | b"none")
    {
        return Err(AdminError::bad_request(
            "requests from other sites are not accepted",
        ));
    }
    if !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
        && headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()) != Some(CSRF_VALUE)
    {
        return Err(AdminError::bad_request(format!(
            "missing `{CSRF_HEADER}: {CSRF_VALUE}` header"
        )));
    }
    Ok(())
}

async fn guard(request: Request, next: Next) -> Response {
    match check_origin(request.method(), request.headers()) {
        Ok(()) => next.run(request).await,
        Err(err) => err.into_response(),
    }
}

/// The authenticated admin user of a request and what they may do. Every
/// handler but `config` and `sign_out` takes it, so they refuse requests that
/// fail to authenticate.
pub(crate) struct Authed {
    pub user: AdminUser,
    pub access: Access,
}

impl FromRequestParts<AdminState> for Authed {
    type Rejection = AdminError;

    async fn from_request_parts(parts: &mut Parts, state: &AdminState) -> AdminResult<Self> {
        let user = state.0.auth.authenticate(&state.0.ctx, parts).await?;
        let access = match user.role {
            None => Access::Full,
            // Without `AdminAuth::roles` the admin does not know the role
            // column, so nothing would stop the user from changing their own
            // role.
            Some(_) if state.0.roles.is_none() => {
                return Err(AdminError::Internal(
                    "the user has a role, but `AdminAuth::roles` returns `None`".to_string(),
                ));
            }
            // A role that no longer exists grants nothing.
            Some(id) => Access::Role(
                roles::find(&state.0.ctx.db, id)
                    .await?
                    .ok_or(AdminError::Forbidden)?,
            ),
        };
        Ok(Self { user, access })
    }
}

/// The request's Loco session; `None` when session cookies cannot be trusted
/// (see [`session::available`]).
pub(crate) struct LocoSession(Option<Session>);

impl FromRequestParts<AdminState> for LocoSession {
    type Rejection = AdminError;

    async fn from_request_parts(parts: &mut Parts, state: &AdminState) -> AdminResult<Self> {
        session::load(&state.0.ctx, parts)
            .await
            .map(Self)
            .map_err(|e| AdminError::Internal(e.to_string()))
    }
}

fn rejection(err: &JsonRejection) -> AdminError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AdminError::PayloadTooLarge
    } else {
        AdminError::bad_request(err.body_text())
    }
}

fn body(
    input: Result<Json<Map<String, JsonValue>>, JsonRejection>,
) -> AdminResult<Map<String, JsonValue>> {
    input.map(|Json(map)| map).map_err(|e| rejection(&e))
}

#[derive(Deserialize)]
pub(crate) struct KeysBody {
    keys: Vec<String>,
}

/// The selected record keys: at least one, and at most one full page, since
/// each key costs a query inside one transaction.
fn keys(input: Result<Json<KeysBody>, JsonRejection>) -> AdminResult<Vec<String>> {
    let Json(body) = input.map_err(|e| rejection(&e))?;
    if body.keys.is_empty() {
        return Err(AdminError::bad_request("no records selected"));
    }
    if body.keys.len() as u64 > MAX_PER_PAGE {
        return Err(AdminError::bad_request(format!(
            "at most {MAX_PER_PAGE} records can be selected at once"
        )));
    }
    Ok(body.keys)
}

pub(crate) fn router() -> Router<AdminState> {
    Router::new()
        .route("/config", get(config))
        .route("/me", get(me))
        .route("/schema", get(schema))
        .route("/stats", get(stats))
        .route("/models/{model}/records", get(list).post(create))
        .route("/models/{model}/count", get(count))
        .route(
            "/models/{model}/records/{key}",
            get(show).patch(update).put(update).delete(destroy),
        )
        .route("/models/{model}/bulk-delete", post(bulk_delete))
        .route("/models/{model}/actions/{action}", post(run_action))
        .route("/models/{model}/export", get(export))
        .route("/models/{model}/lookup", get(lookup))
        .route("/roles", get(list_roles).post(create_role))
        .route(
            "/roles/{id}",
            get(show_role)
                .patch(update_role)
                .put(update_role)
                .delete(delete_role),
        )
        .route("/session", post(sign_in).delete(sign_out))
        .fallback(|| async { AdminError::NotFound })
        .layer(from_fn(guard))
}

/// Title shown in the UI. It is fixed, like the `<title>` in `frontend/index.html`.
const TITLE: &str = "Muy Loco Admin";

async fn config(State(state): State<AdminState>) -> Json<JsonValue> {
    Json(json!({
        "title": TITLE,
        "base_path": state.0.base_path,
        "site_url": state.0.site_url,
        "login": state.0.auth.login(),
    }))
}

/// The user as the UI shows them: their role, and whether they may manage
/// roles.
fn user_json(state: &AdminState, authed: &Authed) -> JsonValue {
    let role = match &authed.access {
        Access::Full => JsonValue::Null,
        Access::Role(role) => json!({ "id": role.id, "name": role.name }),
    };
    json!({
        "id": authed.user.id,
        "name": authed.user.name,
        "role": role,
        "can_manage_roles": state.0.roles.is_some() && authed.access.is_full(),
    })
}

async fn me(State(state): State<AdminState>, authed: Authed) -> Json<JsonValue> {
    Json(user_json(&state, &authed))
}

/// Start a session for the user the request authenticates as: the login form
/// posts the token it got from the app here (as `Authorization: Bearer`) and
/// keeps only the session cookie. Only available with [`LoginMethod::Form`].
async fn sign_in(
    State(state): State<AdminState>,
    authed: AdminResult<Authed>,
    LocoSession(session): LocoSession,
) -> AdminResult<Response> {
    if !matches!(state.0.auth.login(), LoginMethod::Form(_)) {
        return Err(AdminError::NotFound);
    }
    let authed = authed?;
    // Mounting checked that sessions can be trusted.
    let session = session.ok_or_else(|| AdminError::Internal("sessions unavailable".into()))?;
    // A fresh session, so one planted before sign-in is worth nothing.
    let session = session.renew().set_user_id(authed.user.id.clone());
    Ok((session, Json(user_json(&state, &authed))).into_response())
}

/// End the request's session. It is the app's session too, so this signs the
/// user out of both.
async fn sign_out(LocoSession(session): LocoSession) -> Response {
    match session {
        Some(session) => (session.clear(), StatusCode::NO_CONTENT).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn schema(State(state): State<AdminState>, Authed { access, .. }: Authed) -> Json<JsonValue> {
    let models: Vec<Cow<'_, ModelMeta>> = state
        .0
        .models
        .iter()
        .filter_map(|m| state.visible(m, &access))
        .collect();
    Json(json!({ "models": models }))
}

/// How many records each model the user may view has, for the dashboard.
async fn stats(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
) -> AdminResult<Json<JsonValue>> {
    let mut counts = BTreeMap::new();
    for model in &state.0.models {
        if !access.can_view(&model.meta) {
            continue;
        }
        let count = model
            .resource
            .count(&state.0.ctx.db, &ListParams::default())
            .await?;
        counts.insert(model.meta.name.clone(), count);
    }
    Ok(Json(json!({ "counts": counts })))
}

/// Labels of the records that foreign keys in `records` point at, keyed by
/// field name, then by the foreign-key value as text: role names for the
/// role column, and labels from the models the user may view.
async fn fk_labels(
    state: &AdminState,
    model: &Model,
    access: &Access,
    records: &[&Record],
) -> AdminResult<BTreeMap<String, BTreeMap<String, String>>> {
    let mut out = BTreeMap::new();
    for field in &model.meta.fields {
        let values = || {
            records
                .iter()
                .filter_map(|r| r.values.get(&field.name))
                .filter(|v| !v.is_null())
        };
        if field.role {
            let ids: Vec<i64> = values()
                .filter_map(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
                .collect();
            if ids.is_empty() {
                continue;
            }
            let names = roles::names(&state.0.ctx.db, ids).await?;
            out.insert(
                field.name.clone(),
                names
                    .into_iter()
                    .map(|(id, name)| (id.to_string(), name))
                    .collect(),
            );
            continue;
        }
        let Some(rel) = &field.relation else { continue };
        let Some(target_name) = rel.model.as_deref() else {
            continue;
        };
        let Some(target) = state.model(target_name) else {
            continue;
        };
        if !access.can_view(&target.meta) {
            continue;
        }
        let values: Vec<JsonValue> = values().cloned().collect();
        if values.is_empty() {
            continue;
        }
        let labels = target
            .resource
            .labels(&state.0.ctx.db, &rel.column, &values)
            .await?;
        out.insert(field.name.clone(), labels);
    }
    Ok(out)
}

async fn list(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::View)?;
    let params = ListParams::from_query(&query)?;
    let page = model.resource.list(&state.0.ctx.db, &params).await?;
    let records: Vec<&Record> = page.records.iter().collect();
    let labels = fk_labels(&state, model, &access, &records).await?;
    let mut out = serde_json::to_value(&page).map_err(|e| AdminError::Internal(e.to_string()))?;
    out["labels"] = json!(labels);
    Ok(Json(out))
}

/// How many records match a list's search and filters. Lists fetch this
/// apart from their pages, so paging does not count the records again.
async fn count(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::View)?;
    let params = ListParams::from_query(&query)?;
    let count = model.resource.count(&state.0.ctx.db, &params).await?;
    Ok(Json(json!({ "count": count })))
}

async fn record_response(
    state: &AdminState,
    model: &Model,
    access: &Access,
    record: &Record,
) -> AdminResult<Json<JsonValue>> {
    let labels = fk_labels(state, model, access, &[record]).await?;
    Ok(Json(json!({ "record": record, "labels": labels })))
}

async fn show(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path((name, key)): Path<(String, String)>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::View)?;
    let record = model.resource.get(&state.0.ctx.db, &key).await?;
    record_response(&state, model, &access, &record).await
}

async fn create(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(name): Path<String>,
    input: Result<Json<Map<String, JsonValue>>, JsonRejection>,
) -> AdminResult<Response> {
    let model = state.authorize(&access, &name, Operation::Create)?;
    let input = body(input)?;
    let locked = state.locked(model, &access);
    let record = model
        .resource
        .create(&state.0.ctx.db, input, &locked)
        .await?;
    let json = record_response(&state, model, &access, &record).await?;
    Ok((StatusCode::CREATED, json).into_response())
}

async fn update(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path((name, key)): Path<(String, String)>,
    input: Result<Json<Map<String, JsonValue>>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::Update)?;
    let input = body(input)?;
    let locked = state.locked(model, &access);
    let record = model
        .resource
        .update(&state.0.ctx.db, &key, input, &locked)
        .await?;
    record_response(&state, model, &access, &record).await
}

async fn destroy(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path((name, key)): Path<(String, String)>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::Delete)?;
    let deleted = model.resource.delete(&state.0.ctx.db, &[key]).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn bulk_delete(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(name): Path<String>,
    input: Result<Json<KeysBody>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::Delete)?;
    let deleted = model
        .resource
        .delete(&state.0.ctx.db, &keys(input)?)
        .await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn run_action(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path((name, action)): Path<(String, String)>,
    input: Result<Json<KeysBody>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::Action(&action))?;
    let message = model
        .resource
        .run_action(&state.0.ctx, &action, &keys(input)?)
        .await?;
    Ok(Json(json!({ "message": message })))
}

async fn export(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AdminResult<Response> {
    let model = state.authorize(&access, &name, Operation::Export)?;
    let params = ListParams::from_query(&query)?;
    let idle = state.0.export_idle_timeout;
    let csv = Arc::clone(&model.resource)
        .export(&state.0.ctx.db, &params, idle.saturating_mul(2))
        .await?;
    let filename = format!(
        "attachment; filename=\"{}.csv\"",
        model.meta.name.replace('"', "")
    );
    let disposition = HeaderValue::from_str(&filename)
        .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"export.csv\""));
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        spawn_export(csv, idle),
    )
        .into_response())
}

/// Run `csv` in its own task and hand chunks to the response one at a time.
///
/// A stalled download stops the server from reading the response body, so a
/// timeout inside the body would never fire. Here the task waits at most
/// `idle` for the database to produce a chunk and for the client to take it;
/// otherwise it stops, which rolls the export's transaction back and frees
/// its connection. A response that ends early is aborted with an error, so
/// the client never mistakes it for a complete file.
fn spawn_export(mut csv: CsvStream, idle: Duration) -> Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Bytes>(1);
    let finished = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&finished);
    tokio::spawn(async move {
        loop {
            let chunk = match tokio::time::timeout(idle, csv.next()).await {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(None) => return done.store(true, Ordering::Release),
                Ok(Some(Err(err))) => return tracing::error!(error = %err, "CSV export failed"),
                Err(_) => return tracing::warn!("CSV export stopped: no rows from the database"),
            };
            match tokio::time::timeout(idle, tx.send(chunk)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return,
                Err(_) => return tracing::warn!("CSV export stopped: the client stopped reading"),
            }
        }
    });
    Body::from_stream(
        stream::unfold(Some((rx, finished)), |state| async move {
            let (mut rx, finished) = state?;
            match rx.recv().await {
                Some(chunk) => Some((Ok(chunk), Some((rx, finished)))),
                None if finished.load(Ordering::Acquire) => None,
                None => Some((
                    Err(AdminError::Internal(
                        "CSV export did not finish".to_string(),
                    )),
                    None,
                )),
            }
        })
        // Middleware such as compression may poll the body again after its end.
        .fuse(),
    )
}

#[derive(Deserialize)]
pub(crate) struct LookupQuery {
    q: Option<String>,
    column: Option<String>,
    limit: Option<u64>,
}

async fn lookup(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(name): Path<String>,
    Query(query): Query<LookupQuery>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&access, &name, Operation::View)?;
    let limit = query
        .limit
        .unwrap_or(LOOKUP_LIMIT)
        .clamp(1, MAX_LOOKUP_LIMIT);
    let choices = model
        .resource
        .lookup(
            &state.0.ctx.db,
            query.q.as_deref(),
            query.column.as_deref(),
            limit,
        )
        .await?;
    Ok(Json(json!({ "choices": choices })))
}

fn role_json(role: &Role, users: u64) -> JsonValue {
    json!({
        "id": role.id,
        "name": role.name,
        "permissions": role.permissions,
        "users": users,
    })
}

/// A role id from the URL; anything else names no role.
fn role_id(id: &str) -> AdminResult<i64> {
    id.parse().map_err(|_| AdminError::NotFound)
}

fn role_input(input: Result<Json<RoleInput>, JsonRejection>) -> AdminResult<RoleInput> {
    input.map(|Json(input)| input).map_err(|e| rejection(&e))
}

async fn list_roles(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
) -> AdminResult<Json<JsonValue>> {
    let users = state.role_admin(&access)?;
    let db = &state.0.ctx.db;
    let counts = roles::assigned(db, users, None).await?;
    let list: Vec<JsonValue> = roles::all(db)
        .await?
        .iter()
        .map(|role| role_json(role, counts.get(&role.id).copied().unwrap_or(0)))
        .collect();
    Ok(Json(json!({ "roles": list })))
}

async fn show_role(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(id): Path<String>,
) -> AdminResult<Json<JsonValue>> {
    let users = state.role_admin(&access)?;
    let id = role_id(&id)?;
    let db = &state.0.ctx.db;
    let role = roles::find(db, id).await?.ok_or(AdminError::NotFound)?;
    let count = roles::assigned(db, users, Some(id)).await?;
    Ok(Json(
        json!({ "role": role_json(&role, count.get(&id).copied().unwrap_or(0)) }),
    ))
}

async fn create_role(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    input: Result<Json<RoleInput>, JsonRejection>,
) -> AdminResult<Response> {
    state.role_admin(&access)?;
    let role = roles::create(&state.0.ctx.db, &state.metas(), role_input(input)?).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "role": role_json(&role, 0) })),
    )
        .into_response())
}

async fn update_role(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(id): Path<String>,
    input: Result<Json<RoleInput>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let users = state.role_admin(&access)?;
    let id = role_id(&id)?;
    let db = &state.0.ctx.db;
    let role = roles::update(db, &state.metas(), id, role_input(input)?).await?;
    let count = roles::assigned(db, users, Some(id)).await?;
    Ok(Json(
        json!({ "role": role_json(&role, count.get(&id).copied().unwrap_or(0)) }),
    ))
}

async fn delete_role(
    State(state): State<AdminState>,
    Authed { access, .. }: Authed,
    Path(id): Path<String>,
) -> AdminResult<Json<JsonValue>> {
    let users = state.role_admin(&access)?;
    roles::delete(&state.0.ctx.db, users, role_id(&id)?).await?;
    Ok(Json(json!({ "deleted": 1 })))
}

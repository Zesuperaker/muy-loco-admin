//! JSON API handlers.

use std::{
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
    auth::{AdminAuth, AdminUser, LoginMethod},
    error::{AdminError, AdminResult},
    meta::ModelMeta,
    query::{ListParams, MAX_PER_PAGE},
    resource::{CsvStream, Record, Resource},
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
    pub title: String,
    /// Target of the UI's "Visit site" link; `None` hides it.
    pub site_url: Option<String>,
    pub export_idle_timeout: Duration,
    pub base_path: String,
    pub auth: Arc<dyn AdminAuth>,
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
    /// allows `operation`. Every signed-in admin user may do the same.
    fn authorize(&self, name: &str, operation: Operation<'_>) -> AdminResult<&Model> {
        let model = self.model(name).ok_or(AdminError::NotFound)?;
        let caps = model.meta.capabilities;
        let capable = match operation {
            Operation::View => true,
            Operation::Create => caps.create,
            Operation::Update => caps.update,
            Operation::Delete => caps.delete,
            Operation::Export => caps.export,
            Operation::Action(action) => {
                if !model.meta.actions.iter().any(|a| a.name == action) {
                    return Err(AdminError::NotFound);
                }
                true
            }
        };
        if !capable {
            return Err(AdminError::Forbidden);
        }
        Ok(model)
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

/// The authenticated admin user of a request. Every handler but `config`
/// and `sign_out` takes it, so they refuse requests that fail to
/// authenticate.
pub(crate) struct Authed(pub AdminUser);

impl FromRequestParts<AdminState> for Authed {
    type Rejection = AdminError;

    async fn from_request_parts(parts: &mut Parts, state: &AdminState) -> AdminResult<Self> {
        let user = state.0.auth.authenticate(&state.0.ctx, parts).await?;
        Ok(Self(user))
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
        .route(
            "/models/{model}/records/{key}",
            get(show).patch(update).put(update).delete(destroy),
        )
        .route("/models/{model}/bulk-delete", post(bulk_delete))
        .route("/models/{model}/actions/{action}", post(run_action))
        .route("/models/{model}/export", get(export))
        .route("/models/{model}/lookup", get(lookup))
        .route("/session", post(sign_in).delete(sign_out))
        .fallback(|| async { AdminError::NotFound })
        .layer(from_fn(guard))
}

async fn config(State(state): State<AdminState>) -> Json<JsonValue> {
    Json(json!({
        "title": state.0.title,
        "base_path": state.0.base_path,
        "site_url": state.0.site_url,
        "login": state.0.auth.login(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn me(Authed(user): Authed) -> Json<JsonValue> {
    Json(json!({ "id": user.id, "name": user.name }))
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
    let Authed(user) = authed?;
    // Mounting checked that sessions can be trusted.
    let session = session.ok_or_else(|| AdminError::Internal("sessions unavailable".into()))?;
    // A fresh session, so one planted before sign-in is worth nothing.
    let session = session.renew().set_user_id(user.id.clone());
    Ok((session, Json(json!({ "id": user.id, "name": user.name }))).into_response())
}

/// End the request's session. It is the app's session too, so this signs the
/// user out of both.
async fn sign_out(LocoSession(session): LocoSession) -> Response {
    match session {
        Some(session) => (session.clear(), StatusCode::NO_CONTENT).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn schema(State(state): State<AdminState>, _: Authed) -> Json<JsonValue> {
    let models: Vec<&ModelMeta> = state.0.models.iter().map(|m| &m.meta).collect();
    Json(json!({ "models": models }))
}

async fn stats(State(state): State<AdminState>, _: Authed) -> AdminResult<Json<JsonValue>> {
    let mut counts = BTreeMap::new();
    for model in &state.0.models {
        let count = model.resource.count(&state.0.ctx.db).await?;
        counts.insert(model.meta.name.clone(), count);
    }
    Ok(Json(json!({ "counts": counts })))
}

/// Labels of the records that foreign keys in `records` point at, keyed by
/// field name, then by the foreign-key value as text.
async fn fk_labels(
    state: &AdminState,
    model: &Model,
    records: &[&Record],
) -> AdminResult<BTreeMap<String, BTreeMap<String, String>>> {
    let mut out = BTreeMap::new();
    for field in &model.meta.fields {
        let Some(rel) = &field.relation else { continue };
        let Some(target_name) = rel.model.as_deref() else {
            continue;
        };
        let Some(target) = state.model(target_name) else {
            continue;
        };
        let values: Vec<JsonValue> = records
            .iter()
            .filter_map(|r| r.values.get(&field.name).cloned())
            .filter(|v| !v.is_null())
            .collect();
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
    _: Authed,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&name, Operation::View)?;
    let params = ListParams::from_query(&query)?;
    let page = model.resource.list(&state.0.ctx.db, &params).await?;
    let records: Vec<&Record> = page.records.iter().collect();
    let labels = fk_labels(&state, model, &records).await?;
    let mut out = serde_json::to_value(&page).map_err(|e| AdminError::Internal(e.to_string()))?;
    out["labels"] = json!(labels);
    Ok(Json(out))
}

async fn record_response(
    state: &AdminState,
    model: &Model,
    record: &Record,
) -> AdminResult<Json<JsonValue>> {
    let labels = fk_labels(state, model, &[record]).await?;
    Ok(Json(json!({ "record": record, "labels": labels })))
}

async fn show(
    State(state): State<AdminState>,
    _: Authed,
    Path((name, key)): Path<(String, String)>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&name, Operation::View)?;
    let record = model.resource.get(&state.0.ctx.db, &key).await?;
    record_response(&state, model, &record).await
}

async fn create(
    State(state): State<AdminState>,
    _: Authed,
    Path(name): Path<String>,
    input: Result<Json<Map<String, JsonValue>>, JsonRejection>,
) -> AdminResult<Response> {
    let model = state.authorize(&name, Operation::Create)?;
    let record = model.resource.create(&state.0.ctx.db, body(input)?).await?;
    let json = record_response(&state, model, &record).await?;
    Ok((StatusCode::CREATED, json).into_response())
}

async fn update(
    State(state): State<AdminState>,
    _: Authed,
    Path((name, key)): Path<(String, String)>,
    input: Result<Json<Map<String, JsonValue>>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&name, Operation::Update)?;
    let record = model
        .resource
        .update(&state.0.ctx.db, &key, body(input)?)
        .await?;
    record_response(&state, model, &record).await
}

async fn destroy(
    State(state): State<AdminState>,
    _: Authed,
    Path((name, key)): Path<(String, String)>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&name, Operation::Delete)?;
    let deleted = model.resource.delete(&state.0.ctx.db, &[key]).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn bulk_delete(
    State(state): State<AdminState>,
    _: Authed,
    Path(name): Path<String>,
    input: Result<Json<KeysBody>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&name, Operation::Delete)?;
    let deleted = model
        .resource
        .delete(&state.0.ctx.db, &keys(input)?)
        .await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn run_action(
    State(state): State<AdminState>,
    _: Authed,
    Path((name, action)): Path<(String, String)>,
    input: Result<Json<KeysBody>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&name, Operation::Action(&action))?;
    let message = model
        .resource
        .run_action(&state.0.ctx, &action, &keys(input)?)
        .await?;
    Ok(Json(json!({ "message": message })))
}

async fn export(
    State(state): State<AdminState>,
    _: Authed,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AdminResult<Response> {
    let model = state.authorize(&name, Operation::Export)?;
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
    _: Authed,
    Path(name): Path<String>,
    Query(query): Query<LookupQuery>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&name, Operation::View)?;
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

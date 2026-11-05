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
    extract::{FromRequestParts, Path, Query, State, rejection::JsonRejection},
    http::{HeaderValue, Method, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use loco_rs::app::AppContext;
use serde::Deserialize;
use serde_json::{Map, Value as JsonValue, json};

use crate::{
    auth::{Access, AdminAuth, AdminUser, Operation},
    error::{AdminError, AdminResult},
    meta::{Capabilities, ModelMeta},
    query::ListParams,
    resource::{CsvStream, Record, Resource},
};

/// Header every mutating request must carry, so cross-site forms cannot
/// trigger writes when an app authenticates the admin with cookies.
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
    pub export_idle_timeout: Duration,
    pub base_path: String,
    pub auth: Arc<dyn AdminAuth>,
    pub models: Vec<Model>,
    pub ctx: AppContext,
}

#[derive(Clone)]
pub(crate) struct AdminState(pub Arc<Inner>);

impl AdminState {
    fn model(&self, name: &str) -> Option<&Model> {
        self.0.models.iter().find(|m| m.meta.name == name)
    }

    /// Resolve `name`, checking the user may perform `operation` on it.
    /// Models the user cannot view are reported as missing.
    fn authorize(
        &self,
        user: &AdminUser,
        name: &str,
        operation: Operation<'_>,
    ) -> AdminResult<&Model> {
        let model = self.model(name).ok_or(AdminError::NotFound)?;
        if !user.can(&Access {
            model: name,
            operation: Operation::View,
        }) {
            return Err(AdminError::NotFound);
        }
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
        if !capable
            || !user.can(&Access {
                model: name,
                operation,
            })
        {
            return Err(AdminError::Forbidden);
        }
        Ok(model)
    }

    fn can_view(&self, user: &AdminUser, name: &str) -> bool {
        self.model(name).is_some()
            && user.can(&Access {
                model: name,
                operation: Operation::View,
            })
    }
}

/// The authenticated admin user of a request.
pub(crate) struct Authed(pub AdminUser);

impl FromRequestParts<AdminState> for Authed {
    type Rejection = AdminError;

    async fn from_request_parts(parts: &mut Parts, state: &AdminState) -> AdminResult<Self> {
        if !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            let header = parts.headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok());
            if header != Some(CSRF_VALUE) {
                return Err(AdminError::bad_request(format!(
                    "missing `{CSRF_HEADER}: {CSRF_VALUE}` header"
                )));
            }
        }
        let user = state.0.auth.authenticate(&state.0.ctx, parts).await?;
        Ok(Self(user))
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

fn keys(input: Result<Json<KeysBody>, JsonRejection>) -> AdminResult<Vec<String>> {
    let Json(body) = input.map_err(|e| rejection(&e))?;
    if body.keys.is_empty() {
        return Err(AdminError::bad_request("no records selected"));
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
        .fallback(|| async { AdminError::NotFound })
}

async fn config(State(state): State<AdminState>) -> Json<JsonValue> {
    Json(json!({
        "title": state.0.title,
        "base_path": state.0.base_path,
        "login": state.0.auth.login(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn me(Authed(user): Authed) -> Json<JsonValue> {
    Json(json!({ "id": user.id, "name": user.name }))
}

fn user_meta(state: &AdminState, user: &AdminUser, meta: &ModelMeta) -> ModelMeta {
    let mut meta = meta.clone();
    let name = meta.name.clone();
    let can = |operation: Operation<'_>| {
        user.can(&Access {
            model: &name,
            operation,
        })
    };
    let caps = meta.capabilities;
    meta.capabilities = Capabilities {
        view: true,
        create: caps.create && can(Operation::Create),
        update: caps.update && can(Operation::Update),
        delete: caps.delete && can(Operation::Delete),
        export: caps.export && can(Operation::Export),
    };
    meta.actions.retain(|a| can(Operation::Action(&a.name)));
    meta.related.retain(|r| state.can_view(user, &r.model));
    for field in &mut meta.fields {
        if let Some(rel) = &mut field.relation
            && rel
                .model
                .as_deref()
                .is_some_and(|m| !state.can_view(user, m))
        {
            rel.model = None;
        }
    }
    meta
}

async fn schema(State(state): State<AdminState>, Authed(user): Authed) -> Json<JsonValue> {
    let models: Vec<ModelMeta> = state
        .0
        .models
        .iter()
        .filter(|m| state.can_view(&user, &m.meta.name))
        .map(|m| user_meta(&state, &user, &m.meta))
        .collect();
    Json(json!({ "models": models }))
}

async fn stats(
    State(state): State<AdminState>,
    Authed(user): Authed,
) -> AdminResult<Json<JsonValue>> {
    let mut counts = BTreeMap::new();
    for model in &state.0.models {
        if state.can_view(&user, &model.meta.name) {
            let count = model.resource.count(&state.0.ctx.db).await?;
            counts.insert(model.meta.name.clone(), count);
        }
    }
    Ok(Json(json!({ "counts": counts })))
}

/// Labels of the records that foreign keys in `records` point at, keyed by
/// field name, then by the foreign-key value as text.
async fn fk_labels(
    state: &AdminState,
    user: &AdminUser,
    model: &Model,
    records: &[&Record],
) -> AdminResult<BTreeMap<String, BTreeMap<String, String>>> {
    let mut out = BTreeMap::new();
    for field in &model.meta.fields {
        let Some(rel) = &field.relation else { continue };
        let Some(target_name) = rel.model.as_deref() else {
            continue;
        };
        if !state.can_view(user, target_name) {
            continue;
        }
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
    Authed(user): Authed,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&user, &name, Operation::View)?;
    let params = ListParams::from_query(&query)?;
    let page = model.resource.list(&state.0.ctx.db, &params).await?;
    let records: Vec<&Record> = page.records.iter().collect();
    let labels = fk_labels(&state, &user, model, &records).await?;
    let mut out = serde_json::to_value(&page).map_err(|e| AdminError::Internal(e.to_string()))?;
    out["labels"] = json!(labels);
    Ok(Json(out))
}

async fn record_response(
    state: &AdminState,
    user: &AdminUser,
    model: &Model,
    record: &Record,
) -> AdminResult<Json<JsonValue>> {
    let labels = fk_labels(state, user, model, &[record]).await?;
    Ok(Json(json!({ "record": record, "labels": labels })))
}

async fn show(
    State(state): State<AdminState>,
    Authed(user): Authed,
    Path((name, key)): Path<(String, String)>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&user, &name, Operation::View)?;
    let record = model.resource.get(&state.0.ctx.db, &key).await?;
    record_response(&state, &user, model, &record).await
}

async fn create(
    State(state): State<AdminState>,
    Authed(user): Authed,
    Path(name): Path<String>,
    input: Result<Json<Map<String, JsonValue>>, JsonRejection>,
) -> AdminResult<Response> {
    let model = state.authorize(&user, &name, Operation::Create)?;
    let record = model.resource.create(&state.0.ctx.db, body(input)?).await?;
    let json = record_response(&state, &user, model, &record).await?;
    Ok((StatusCode::CREATED, json).into_response())
}

async fn update(
    State(state): State<AdminState>,
    Authed(user): Authed,
    Path((name, key)): Path<(String, String)>,
    input: Result<Json<Map<String, JsonValue>>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&user, &name, Operation::Update)?;
    let record = model
        .resource
        .update(&state.0.ctx.db, &key, body(input)?)
        .await?;
    record_response(&state, &user, model, &record).await
}

async fn destroy(
    State(state): State<AdminState>,
    Authed(user): Authed,
    Path((name, key)): Path<(String, String)>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&user, &name, Operation::Delete)?;
    let deleted = model.resource.delete(&state.0.ctx.db, &[key]).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn bulk_delete(
    State(state): State<AdminState>,
    Authed(user): Authed,
    Path(name): Path<String>,
    input: Result<Json<KeysBody>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&user, &name, Operation::Delete)?;
    let deleted = model
        .resource
        .delete(&state.0.ctx.db, &keys(input)?)
        .await?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn run_action(
    State(state): State<AdminState>,
    Authed(user): Authed,
    Path((name, action)): Path<(String, String)>,
    input: Result<Json<KeysBody>, JsonRejection>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&user, &name, Operation::Action(&action))?;
    let message = model
        .resource
        .run_action(&state.0.ctx, &action, &keys(input)?)
        .await?;
    Ok(Json(json!({ "message": message })))
}

async fn export(
    State(state): State<AdminState>,
    Authed(user): Authed,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AdminResult<Response> {
    let model = state.authorize(&user, &name, Operation::Export)?;
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
    Authed(user): Authed,
    Path(name): Path<String>,
    Query(query): Query<LookupQuery>,
) -> AdminResult<Json<JsonValue>> {
    let model = state.authorize(&user, &name, Operation::View)?;
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

mod common;

use std::path::Path;

use async_trait::async_trait;
use axum::{
    Router,
    http::{HeaderValue, Method},
    middleware::map_response,
    response::Response,
};
use common::{Client, TestDb, context, database, posts, users};
use loco_rs::{
    Result,
    app::{AppContext, Hooks, Initializer},
    bgworker::Queue,
    boot::{BootResult, StartMode},
    config::Config,
    controller::{AppRoutes, middleware::MiddlewareLayer},
    environment::Environment,
    task::Tasks,
};
use muy_loco_admin::{Admin, AllowAll};
use serde_json::{Value, json};

/// A middleware an app could add in `Hooks::middlewares`.
struct Stamp;

impl MiddlewareLayer for Stamp {
    fn name(&self) -> &'static str {
        "stamp"
    }

    fn config(&self) -> serde_json::Result<Value> {
        Ok(json!({}))
    }

    fn apply(&self, app: Router<AppContext>) -> Result<Router<AppContext>> {
        Ok(app.layer(map_response(|mut res: Response| async move {
            res.headers_mut()
                .insert("x-stamp", HeaderValue::from_static("app"));
            res
        })))
    }
}

/// An app whose middleware stack is only [`Stamp`].
struct StampApp;

#[async_trait]
impl Hooks for StampApp {
    fn app_name() -> &'static str {
        "stamp-app"
    }

    async fn boot(_mode: StartMode, _env: &Environment, _config: Config) -> Result<BootResult> {
        Err(loco_rs::Error::string("not used"))
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        AppRoutes::empty()
    }

    fn middlewares(_ctx: &AppContext) -> Vec<Box<dyn MiddlewareLayer>> {
        vec![Box::new(Stamp)]
    }

    async fn connect_workers(_ctx: &AppContext, _queue: &Queue) -> Result<()> {
        Ok(())
    }

    fn register_tasks(_tasks: &mut Tasks) {}

    async fn truncate(_ctx: &AppContext) -> Result<()> {
        Ok(())
    }

    async fn seed(_ctx: &AppContext, _path: &Path) -> Result<()> {
        Ok(())
    }
}

fn admin() -> Admin {
    Admin::new(AllowAll)
        .model::<users::Entity>()
        .model::<posts::Entity>()
}

async fn mounted(test_db: &TestDb, admin: Admin, middlewares: Value) -> Client {
    let ctx = context(
        test_db,
        Environment::Test,
        &json!({"server": {"port": 5150, "host": "http://localhost", "middlewares": middlewares}}),
    );
    let app = Router::new().fallback(|| async { "app fallback" });
    Client::new(admin.after_routes(app, &ctx).await.unwrap())
}

#[tokio::test]
async fn default_stack_wraps_admin_requests() {
    let test_db = database().await;
    let client = mounted(&test_db, admin(), json!({})).await;
    let res = client.get("/admin/api/config").await;
    assert_eq!(res.status, 200);
    assert!(res.header("x-request-id").is_some(), "request_id applies");
    assert_eq!(res.header("x-powered-by"), Some("loco.rs"));
    assert!(res.header("x-stamp").is_none());
}

#[tokio::test]
async fn config_switches_middleware_on_and_off() {
    let test_db = database().await;
    let client = mounted(
        &test_db,
        admin(),
        json!({"request_id": {"enable": false}, "compression": {"enable": true}}),
    )
    .await;
    let res = client
        .send_raw(
            Method::GET,
            "/admin",
            None,
            vec![("accept-encoding", "gzip")],
        )
        .await;
    assert_eq!(res.status, 200);
    assert!(res.header("x-request-id").is_none(), "disabled in config");
    assert_eq!(res.header("content-encoding"), Some("gzip"));
}

#[tokio::test]
async fn app_middleware_stack_is_used() {
    let test_db = database().await;
    let client = mounted(&test_db, admin().middlewares_from::<StampApp>(), json!({})).await;
    let res = client.get("/admin/api/schema").await;
    assert_eq!(res.status, 200);
    assert_eq!(
        res.header("x-stamp"),
        Some("app"),
        "custom middleware applies"
    );
    assert!(
        res.header("x-request-id").is_none(),
        "middleware the app removed stays off"
    );
    assert_eq!(client.get("/").await.text(), "app fallback");
}

#[tokio::test]
async fn route_installing_middleware_is_skipped() {
    let test_db = database().await;
    let client = mounted(
        &test_db,
        admin(),
        json!({
            "fallback": {"enable": true, "code": 200},
            "static": {
                "enable": true,
                "must_exist": false,
                "precompressed": false,
                "folder": {"uri": "/static", "path": "missing"},
                "fallback": "missing/404.html"
            }
        }),
    )
    .await;
    let res = client.get("/admin/models/posts").await;
    assert_eq!(res.status, 200);
    assert_eq!(res.header("content-type"), Some("text/html; charset=utf-8"));
    let res = client.get("/admin/api/unknown").await;
    assert_eq!(res.json(), json!({"error": "not found"}));
}

#[tokio::test]
async fn admin_headers_win_over_secure_headers_presets() {
    let test_db = database().await;
    for preset in ["github", "owasp"] {
        let client = mounted(
            &test_db,
            admin(),
            json!({"secure_headers": {"enable": true, "preset": preset}}),
        )
        .await;
        let res = client.get("/admin").await;
        assert_eq!(res.status, 200);
        let csp = res.header("content-security-policy").unwrap();
        assert!(csp.contains("script-src 'self'"), "{preset}: {csp}");
        assert_eq!(res.header("x-frame-options"), Some("DENY"));
        assert!(
            res.header("strict-transport-security").is_some(),
            "{preset}: app headers kept"
        );
        assert!(res.header("clear-site-data").is_none(), "{preset}");
    }
}

#[tokio::test]
async fn payload_limit_applies() {
    let test_db = database().await;
    let client = mounted(
        &test_db,
        admin(),
        json!({"limit_payload": {"body_limit": "1kb"}}),
    )
    .await;
    let body = json!({"title": "x".repeat(4096)}).to_string();
    let res = client
        .send_raw(
            Method::POST,
            "/admin/api/models/posts/records",
            Some(body),
            vec![],
        )
        .await;
    assert_eq!(res.status, 413);
    assert_eq!(res.json()["error"], "request body is too large");
    let keys = json!({"keys": ["x".repeat(4096)]});
    let res = client
        .post("/admin/api/models/posts/bulk-delete", keys)
        .await;
    assert_eq!(res.status, 413);
}

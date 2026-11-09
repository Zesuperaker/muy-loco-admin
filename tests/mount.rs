mod common;

use axum::{Router, http::Method, routing::get};
use common::{Client, context, database, loco_sessions, posts, users};
use loco_rs::{app::Initializer, environment::Environment};
use muy_loco_admin::{Admin, AllowAll, CONFIG_KEY};
use serde_json::json;

fn admin() -> Admin {
    Admin::new(AllowAll)
        .model::<users::Entity>()
        .model::<posts::Entity>()
}

fn app_router() -> Router {
    Router::new()
        .route("/", get(|| async { "app home" }))
        .fallback(|| async { "app fallback" })
}

#[tokio::test]
async fn initializer_mounts_the_admin() {
    let test_db = database().await;
    let ctx = context(&test_db, Environment::Test, &json!({}));
    let admin = admin();
    assert_eq!(admin.name(), CONFIG_KEY);
    assert_eq!(admin.mount_path().unwrap(), "/admin");
    let router = admin.after_routes(app_router(), &ctx).await.unwrap();
    let client = Client::new(router);

    assert_eq!(client.get("/").await.text(), "app home");
    assert_eq!(client.get("/elsewhere").await.text(), "app fallback");

    let res = client.get("/admin/api/config").await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["base_path"], "/admin");
    assert_eq!(res.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(res.header("x-frame-options"), Some("DENY"));
    assert_eq!(res.header("referrer-policy"), Some("same-origin"));
    assert_eq!(res.header("cache-control"), Some("no-store"));

    let res = client.get("/admin/").await;
    assert_eq!(res.status, 308);
    assert_eq!(res.header("location"), Some("/admin"));

    for uri in [
        "/admin",
        "/admin/models/posts",
        "/admin/models/posts/records/1",
    ] {
        let res = client.get(uri).await;
        assert_eq!(res.status, 200, "{uri}");
        assert_eq!(
            res.header("content-type"),
            Some("text/html; charset=utf-8"),
            "{uri}"
        );
        assert!(
            res.header("content-security-policy")
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
        assert_eq!(res.header("cache-control"), Some("no-cache"));
        let html = res.text();
        assert!(!html.contains("__MLA_BASE__"));
    }

    let res = client.send(Method::POST, "/admin/models", None).await;
    assert_eq!(res.status, 404);
    let res = client.get("/admin/assets/missing.js").await;
    assert_eq!(res.status, 404);
    let res = client.get("/admin/api/unknown").await;
    assert_eq!(res.json(), json!({"error": "not found"}));
}

#[tokio::test]
async fn embedded_assets_are_served() {
    let test_db = database().await;
    let ctx = context(&test_db, Environment::Test, &json!({}));
    let router = admin().after_routes(app_router(), &ctx).await.unwrap();
    let client = Client::new(router);
    let html = client.get("/admin").await.text();
    let Some(start) = html.find("/admin/assets/") else {
        // The UI has not been built; the placeholder page is served instead.
        assert!(html.contains("has not been built"));
        return;
    };
    let rest = &html[start..];
    let end = rest.find('"').unwrap();
    let res = client.get(&rest[..end]).await;
    assert_eq!(res.status, 200);
    assert_eq!(
        res.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert!(res.header("content-type").unwrap().starts_with("text/"));
}

#[tokio::test]
async fn config_overrides_path_title_and_site_url() {
    let test_db = database().await;
    let ctx = context(
        &test_db,
        Environment::Test,
        &json!({"initializers": {"admin": {
            "path": "/backoffice/",
            "title": "Back office",
            "site_url": "https://example.com/",
        }}}),
    );
    let router = admin()
        .site_url("/home")
        .after_routes(app_router(), &ctx)
        .await
        .unwrap();
    let client = Client::new(router);
    let res = client.get("/backoffice/api/config").await;
    assert_eq!(res.json()["title"], "Back office");
    assert_eq!(res.json()["base_path"], "/backoffice");
    assert_eq!(res.json()["site_url"], "https://example.com/");
    assert_eq!(client.get("/admin/api/config").await.text(), "app fallback");

    let ctx = context(
        &test_db,
        Environment::Test,
        &json!({"initializers": {"admin": {"site_url": " "}}}),
    );
    let router = admin().after_routes(app_router(), &ctx).await.unwrap();
    let res = Client::new(router).get("/admin/api/config").await;
    assert_eq!(res.json()["site_url"], serde_json::Value::Null);
}

#[tokio::test]
async fn config_can_disable_the_admin() {
    let test_db = database().await;
    let ctx = context(
        &test_db,
        Environment::Test,
        &json!({"initializers": {"admin": {"enabled": false}}}),
    );
    let router = admin().after_routes(app_router(), &ctx).await.unwrap();
    let client = Client::new(router);
    assert_eq!(client.get("/admin/api/config").await.text(), "app fallback");
}

#[tokio::test]
async fn invalid_config_is_rejected() {
    let test_db = database().await;
    for settings in [
        json!({"pth": "/x"}),
        json!({"path": "/"}),
        json!({"site_url": "javascript:alert(1)"}),
        json!("yes"),
    ] {
        let ctx = context(
            &test_db,
            Environment::Test,
            &json!({"initializers": {"admin": settings}}),
        );
        assert!(
            admin().after_routes(app_router(), &ctx).await.is_err(),
            "{settings}"
        );
    }
}

#[tokio::test]
async fn router_can_be_mounted_by_hand() {
    let test_db = database().await;
    let ctx = context(&test_db, Environment::Test, &json!({}));
    let admin = admin().path("/ops/admin").title("Ops").site_url("/home");
    let path = admin.mount_path().unwrap();
    let router = Router::new().nest(&path, admin.router(&ctx).await.unwrap());
    let res = Client::new(router).get("/ops/admin/api/config").await;
    assert_eq!(res.json()["title"], "Ops");
    assert_eq!(res.json()["base_path"], "/ops/admin");
    assert_eq!(res.json()["site_url"], "/home");
    assert!(Admin::new(AllowAll).path("nope").mount_path().is_err());
}

#[tokio::test]
async fn loco_session_tables_cannot_be_registered() {
    let test_db = database().await;
    let ctx = context(&test_db, Environment::Test, &json!({}));
    let err = admin()
        .model::<loco_sessions::Entity>()
        .router(&ctx)
        .await
        .expect_err("session table registered");
    assert!(
        err.to_string()
            .contains("the `loco_sessions` table holds Loco's sessions"),
        "{err}"
    );
}

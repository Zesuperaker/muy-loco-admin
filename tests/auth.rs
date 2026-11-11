mod common;

use async_trait::async_trait;
use axum::http::{Method, request::Parts};
use common::{
    Client, JWT_SECRET, context, create_post, create_user, database, posts, setup, users,
};
use loco_rs::{app::AppContext, auth::jwt::JWT, environment::Environment};
use muy_loco_admin::{
    Admin, AdminAuth, AdminUser, AllowAll, AuthError, FormLogin, LoginMethod, ModelAdmin,
    auth::JwtAuth,
};
use serde_json::{Map, json};

fn token(pid: &str) -> String {
    JWT::new(JWT_SECRET)
        .generate_token(3600, pid.to_string(), Map::new())
        .expect("token")
}

fn jwt_admin() -> Admin {
    Admin::new(
        JwtAuth::<users::Model>::new(|user| user.is_admin).display_name(|user| user.email.clone()),
    )
    .model::<users::Entity>()
    .register(
        ModelAdmin::<posts::Entity>::new()
            .action("noop", "Noop", |_, _| async { Ok("done".to_string()) }),
    )
}

#[tokio::test]
async fn jwt_auth_flow() {
    let (db, ctx) = setup().await;
    let router = jwt_admin().router(&ctx).await.unwrap();
    let anonymous = Client::new(router.clone());

    let config = anonymous.get("/api/config").await.json();
    assert_eq!(config["login"]["type"], "form");
    assert_eq!(config["login"]["endpoint"], "/api/auth/login");

    assert_eq!(anonymous.get("/api/me").await.status, 401);
    assert_eq!(
        anonymous.get("/api/schema").await.json()["error"],
        "authentication required"
    );
    let garbage = anonymous.clone().with_token("not-a-jwt");
    assert_eq!(garbage.get("/api/me").await.status, 401);
    let unknown = anonymous
        .clone()
        .with_token(token("00000000-0000-0000-0000-00000000ffff"));
    assert_eq!(unknown.get("/api/me").await.status, 401);
    let broken = anonymous.clone().with_token(token("explode"));
    assert_eq!(broken.get("/api/me").await.status, 500);

    let regular = create_user(&db.db, "joe@example.com", false).await;
    let joe = anonymous
        .clone()
        .with_token(token(&regular.pid.to_string()));
    assert_eq!(joe.get("/api/me").await.status, 403);

    let staff = create_user(&db.db, "staff@example.com", true).await;
    let staff_client = anonymous.clone().with_token(token(&staff.pid.to_string()));
    let me = staff_client.get("/api/me").await.json();
    assert_eq!(
        me,
        json!({
            "id": staff.pid.to_string(),
            "name": "staff@example.com",
            "role": null,
            "can_manage_roles": false
        })
    );

    // Every admin user may do everything the models allow.
    let post = create_post(&db.db, staff.id, "p").await;
    let schema = staff_client.get("/api/schema").await.json();
    assert_eq!(
        schema["models"][1]["capabilities"],
        json!({"view": true, "create": true, "update": true, "delete": true, "export": true})
    );
    assert_eq!(schema["models"][1]["actions"][0]["name"], "noop");
    let res = staff_client
        .patch(
            &format!("/api/models/posts/records/{}", post.id),
            json!({"title": "x"}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    let res = staff_client
        .post(
            "/api/models/posts/actions/noop",
            json!({"keys": [post.id.to_string()]}),
        )
        .await;
    assert_eq!(res.json()["message"], "done");
    assert_eq!(
        staff_client.get("/api/models/posts/export").await.status,
        200
    );
}

#[tokio::test]
async fn jwt_token_from_configured_cookie() {
    let test_db = database().await;
    let ctx = context(
        &test_db,
        Environment::Test,
        &json!({"auth": {"jwt": {
            "secret": JWT_SECRET,
            "expiration": 3600,
            "location": {"from": "Cookie", "name": "session"}
        }}}),
    );
    let router = Admin::new(JwtAuth::<users::Model>::new(|_| true))
        .model::<users::Entity>()
        .router(&ctx)
        .await
        .unwrap();
    let user = create_user(&test_db.db, "ann@example.com", false).await;
    let cookie = format!("session={}", token(&user.pid.to_string()));
    let res = Client::new(router.clone())
        .send_raw(Method::GET, "/api/me", None, vec![("cookie", &cookie)])
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(
        res.json()["name"],
        user.pid.to_string(),
        "defaults to the pid"
    );

    let bearer = Client::new(router).with_token(token(&user.pid.to_string()));
    assert_eq!(
        bearer.get("/api/me").await.status,
        200,
        "bearer header still works"
    );
}

#[tokio::test]
async fn jwt_without_config_is_an_error() {
    let test_db = database().await;
    let ctx = context(&test_db, Environment::Test, &json!({"auth": null}));
    let router = Admin::new(JwtAuth::<users::Model>::new(|_| true).login_method(
        LoginMethod::Redirect {
            url: "/login".into(),
        },
    ))
    .model::<users::Entity>()
    .router(&ctx)
    .await
    .unwrap();
    let client = Client::new(router).with_token("x");
    assert_eq!(client.get("/api/me").await.status, 500);
    let config = client.get("/api/config").await.json();
    assert_eq!(
        config["login"],
        json!({"type": "redirect", "url": "/login"})
    );
}

#[tokio::test]
async fn login_endpoint_moves_the_login_form() {
    let (_db, ctx) = setup().await;
    let ctx = &ctx;
    let login = |auth: JwtAuth<users::Model>| async move {
        let router = Admin::new(auth)
            .model::<users::Entity>()
            .router(ctx)
            .await
            .unwrap();
        Client::new(router).get("/api/config").await.json()["login"].clone()
    };
    let jwt = || JwtAuth::<users::Model>::new(|_| true);

    assert_eq!(
        login(jwt().login_endpoint("/auth/sign_in")).await,
        json!({
            "type": "form",
            "endpoint": "/auth/sign_in",
            "identifier_field": "email",
            "identifier_label": "Email",
            "password_field": "password",
            "token_field": "token",
        })
    );

    let username = jwt()
        .login_method(LoginMethod::Form(FormLogin {
            identifier_field: "username".into(),
            ..FormLogin::default()
        }))
        .login_endpoint("/session");
    let form = login(username).await;
    assert_eq!(form["endpoint"], "/session");
    assert_eq!(form["identifier_field"], "username", "other settings kept");

    let redirect = jwt()
        .login_method(LoginMethod::Redirect {
            url: "/login".into(),
        })
        .login_endpoint("/session");
    let form = login(redirect).await;
    assert_eq!(form["type"], "form");
    assert_eq!(form["identifier_field"], "email");
}

#[tokio::test]
async fn allow_all_only_mounts_in_development_and_test() {
    let test_db = database().await;
    for env in [Environment::Development, Environment::Test] {
        let ctx = context(&test_db, env.clone(), &json!({}));
        let router = Admin::new(AllowAll)
            .model::<users::Entity>()
            .router(&ctx)
            .await
            .unwrap();
        let res = Client::new(router).get("/api/me").await;
        assert_eq!(res.status, 200, "{env:?}");
    }
    for env in [
        Environment::Production,
        Environment::Any("staging".into()),
        Environment::Any("prod".into()),
    ] {
        let ctx = context(&test_db, env.clone(), &json!({}));
        let err = Admin::new(AllowAll)
            .model::<users::Entity>()
            .router(&ctx)
            .await
            .expect_err("refuses to mount")
            .to_string();
        assert!(err.contains("AllowAll"), "{env:?}: {err}");
        assert!(err.contains(&format!("`{env}`")), "{env:?}: {err}");
    }
}

/// Auth that delegates to `AllowAll` but not its mount check.
struct Wrapped;

#[async_trait]
impl AdminAuth for Wrapped {
    async fn authenticate(&self, ctx: &AppContext, parts: &Parts) -> Result<AdminUser, AuthError> {
        AllowAll.authenticate(ctx, parts).await
    }
}

#[tokio::test]
async fn allow_all_still_refuses_requests_where_it_cannot_mount() {
    let test_db = database().await;
    let ctx = context(&test_db, Environment::Production, &json!({}));
    let router = Admin::new(Wrapped)
        .model::<users::Entity>()
        .router(&ctx)
        .await
        .unwrap();
    let res = Client::new(router).get("/api/me").await;
    assert_eq!(res.status, 403);
}

/// A custom auth reading a header.
struct HeaderAuth;

#[async_trait]
impl AdminAuth for HeaderAuth {
    async fn authenticate(&self, _ctx: &AppContext, parts: &Parts) -> Result<AdminUser, AuthError> {
        match parts.headers.get("x-role").and_then(|v| v.to_str().ok()) {
            Some("editor") => Ok(AdminUser::new("1", "Editor")),
            Some("broken") => Err(AuthError::Internal("auth backend down".into())),
            Some(_) => Err(AuthError::Forbidden),
            None => Err(AuthError::Unauthenticated),
        }
    }

    fn login(&self) -> LoginMethod {
        LoginMethod::Form(FormLogin {
            endpoint: "/session".into(),
            identifier_field: "username".into(),
            identifier_label: "Username".into(),
            ..FormLogin::default()
        })
    }
}

#[tokio::test]
async fn custom_auth() {
    let (db, ctx) = setup().await;
    let router = Admin::new(HeaderAuth)
        .model::<users::Entity>()
        .model::<posts::Entity>()
        .router(&ctx)
        .await
        .unwrap();
    let client = Client::new(router);
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let post = create_post(&db.db, ann.id, "p").await;
    let editor = |method: Method, uri: String| {
        let client = client.clone();
        async move {
            client
                .send_raw(method, &uri, None, vec![("x-role", "editor")])
                .await
        }
    };

    let config = client.get("/api/config").await.json();
    assert_eq!(config["login"]["endpoint"], "/session");
    assert_eq!(config["login"]["identifier_field"], "username");

    let res = client
        .send_raw(Method::GET, "/api/me", None, vec![("x-role", "viewer")])
        .await;
    assert_eq!(res.status, 403);
    let res = client
        .send_raw(Method::GET, "/api/me", None, vec![("x-role", "broken")])
        .await;
    assert_eq!(res.status, 500);
    assert_eq!(res.json()["error"], "internal server error");

    let schema = editor(Method::GET, "/api/schema".into()).await.json();
    let models = schema["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(
        models[1]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "user_id")
            .unwrap()["relation"]["model"],
        "users"
    );

    let stats = editor(Method::GET, "/api/stats".into()).await.json();
    assert_eq!(
        stats["counts"],
        json!({"posts": common::exact(1), "users": common::exact(1)})
    );
    let list = editor(Method::GET, "/api/models/posts/records".into())
        .await
        .json();
    assert_eq!(list["labels"]["user_id"][ann.id.to_string()], "ann");
    let res = client
        .send_raw(
            Method::DELETE,
            &format!("/api/models/posts/records/{}", post.id),
            None,
            vec![("x-role", "editor"), ("x-requested-with", "muy-loco-admin")],
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(
        client.get("/api/stats").await.status,
        401,
        "no header, no access"
    );
}

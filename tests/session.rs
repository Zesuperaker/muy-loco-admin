mod common;

use axum::http::Method;
use common::{Client, JWT_SECRET, Res, TestDb, context, create_user, database, setup, users};
use loco_rs::{auth::jwt::JWT, environment::Environment};
use muy_loco_admin::{Admin, LoginMethod, auth::JwtAuth};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
use serde_json::{Map, Value, json};

const SESSION_SECRET: &str = "a-test-session-secret-that-is-long-enough-0123456789";

fn token(pid: &str) -> String {
    JWT::new(JWT_SECRET)
        .generate_token(3600, pid.to_string(), Map::new())
        .expect("token")
}

fn admin() -> Admin {
    Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin).display_name(|u| u.email.clone()))
        .model::<users::Entity>()
}

/// Config with `auth.session` set to `session` (and `server`, if given).
fn config(session: Value, server: Option<Value>) -> Value {
    let mut config = json!({
        "auth": { "jwt": { "secret": JWT_SECRET, "expiration": 3600 } }
    });
    config["auth"]["session"] = session;
    if let Some(server) = server {
        config["server"] = server;
    }
    config
}

fn mount(test_db: &TestDb, env: Environment, session: Value) -> Result<Client, String> {
    let ctx = context(test_db, env, &config(session, None));
    admin()
        .router(&ctx)
        .map(Client::new)
        .map_err(|e| e.to_string())
}

/// The `Set-Cookie` header of `res`.
fn set_cookie(res: &Res) -> &str {
    res.header("set-cookie").expect("a Set-Cookie header")
}

/// Sign `user` in through the login form's exchange; the session cookie.
async fn sign_in(client: &Client, user: &users::Model) -> String {
    let res = client
        .clone()
        .with_token(token(&user.pid.to_string()))
        .send(Method::POST, "/api/session", None)
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    set_cookie(&res).split(';').next().unwrap().to_string()
}

async fn send(client: &Client, method: Method, uri: &str, cookie: &str) -> Res {
    client
        .send_raw(method, uri, None, vec![("cookie", cookie)])
        .await
}

#[tokio::test]
async fn the_login_form_trades_its_token_for_a_session_cookie() {
    let (db, ctx) = setup().await;
    let client = Client::new(admin().router(&ctx).unwrap());
    let staff = create_user(&db.db, "staff@example.com", true).await;

    let res = client
        .clone()
        .with_token(token(&staff.pid.to_string()))
        .send(Method::POST, "/api/session", None)
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(
        res.json(),
        json!({"id": staff.pid.to_string(), "name": "staff@example.com"})
    );
    let cookie = set_cookie(&res);
    assert!(cookie.starts_with("loco_session="), "{cookie}");
    for attribute in ["HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(cookie.contains(attribute), "{attribute}: {cookie}");
    }
    assert!(!cookie.contains("Secure"), "plain-http host: {cookie}");
    let cookie = cookie.split(';').next().unwrap();

    let res = send(&client, Method::GET, "/api/me", cookie).await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(res.json()["name"], "staff@example.com");

    let res = client
        .clone()
        .without_csrf()
        .send_raw(
            Method::DELETE,
            "/api/session",
            None,
            vec![("cookie", cookie)],
        )
        .await;
    assert_eq!(res.status, 400, "writes need the CSRF header");

    let res = send(&client, Method::DELETE, "/api/session", cookie).await;
    assert_eq!(res.status, 204);
    let removal = set_cookie(&res);
    assert!(removal.starts_with("loco_session=;"), "{removal}");
    assert!(removal.contains("Max-Age=0"), "{removal}");
}

#[tokio::test]
async fn sign_in_needs_an_admin_and_valid_credentials() {
    let (db, ctx) = setup().await;
    let client = Client::new(admin().router(&ctx).unwrap());
    let regular = create_user(&db.db, "joe@example.com", false).await;

    let res = client.send(Method::POST, "/api/session", None).await;
    assert_eq!(res.status, 401);
    assert!(res.header("set-cookie").is_none());

    let res = client
        .clone()
        .with_token("not-a-jwt")
        .send(Method::POST, "/api/session", None)
        .await;
    assert_eq!(res.status, 401);

    let res = client
        .clone()
        .with_token(token(&regular.pid.to_string()))
        .send(Method::POST, "/api/session", None)
        .await;
    assert_eq!(res.status, 403);
    assert!(
        res.header("set-cookie").is_none(),
        "no session for non-admins"
    );
}

#[tokio::test]
async fn sessions_are_rechecked_on_every_request() {
    let (db, ctx) = setup().await;
    let client = Client::new(admin().router(&ctx).unwrap());
    let staff = create_user(&db.db, "staff@example.com", true).await;
    let cookie = sign_in(&client, &staff).await;
    assert_eq!(
        send(&client, Method::GET, "/api/me", &cookie).await.status,
        200
    );

    users::Entity::update_many()
        .col_expr(users::Column::IsAdmin, Expr::value(false))
        .filter(users::Column::Id.eq(staff.id))
        .exec(&db.db)
        .await
        .unwrap();
    assert_eq!(
        send(&client, Method::GET, "/api/me", &cookie).await.status,
        403,
        "admin access revoked"
    );

    users::Entity::delete_by_id(staff.id)
        .exec(&db.db)
        .await
        .unwrap();
    assert_eq!(
        send(&client, Method::GET, "/api/me", &cookie).await.status,
        401,
        "user deleted"
    );
}

#[tokio::test]
async fn server_side_sessions_end_on_sign_out() {
    let test_db = database().await;
    let client = mount(&test_db, Environment::Test, json!({"store": "database"})).unwrap();
    let staff = create_user(&test_db.db, "staff@example.com", true).await;
    let cookie = sign_in(&client, &staff).await;
    assert_eq!(
        send(&client, Method::GET, "/api/me", &cookie).await.status,
        200
    );

    let res = send(&client, Method::DELETE, "/api/session", &cookie).await;
    assert_eq!(res.status, 204);
    assert_eq!(
        send(&client, Method::GET, "/api/me", &cookie).await.status,
        401,
        "a copied cookie stops working after sign-out"
    );
}

#[tokio::test]
async fn https_hosts_get_secure_host_only_cookies() {
    let test_db = database().await;
    let ctx = context(
        &test_db,
        Environment::Production,
        &config(
            json!({"secret": SESSION_SECRET, "cookie_name": "__Host-loco_session"}),
            Some(json!({"port": 5150, "host": "https://admin.example.com"})),
        ),
    );
    let client = Client::new(admin().router(&ctx).unwrap());
    let staff = create_user(&test_db.db, "staff@example.com", true).await;
    let res = client
        .clone()
        .with_token(token(&staff.pid.to_string()))
        .send(Method::POST, "/api/session", None)
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    let cookie = set_cookie(&res);
    assert!(cookie.starts_with("__Host-loco_session="), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    assert!(!cookie.contains("Domain"), "{cookie}");
    let cookie = cookie.split(';').next().unwrap();
    assert_eq!(
        send(&client, Method::GET, "/api/me", cookie).await.status,
        200
    );
}

#[tokio::test]
async fn mounting_checks_the_session_setup() {
    let test_db = database().await;
    for env in [Environment::Production, Environment::Any("staging".into())] {
        let err = mount(&test_db, env.clone(), json!({})).err().unwrap();
        assert!(err.contains("auth.session.secret"), "{env:?}: {err}");
        mount(&test_db, env, json!({"secret": SESSION_SECRET})).unwrap();
    }
    mount(&test_db, Environment::Development, json!({})).unwrap();

    for name in ["__Host-loco_session", "__secure-loco_session"] {
        let err = mount(&test_db, Environment::Test, json!({"cookie_name": name}))
            .err()
            .unwrap();
        assert!(err.contains("Secure"), "{name}: {err}");
        mount(
            &test_db,
            Environment::Test,
            json!({"cookie_name": name, "secure": true}),
        )
        .unwrap();
    }

    // Only the login form starts sessions; other login methods need no setup.
    let ctx = context(&test_db, Environment::Production, &config(json!({}), None));
    let redirect = JwtAuth::<users::Model>::new(|_| true).login_method(LoginMethod::Redirect {
        url: "/login".into(),
    });
    let client = Client::new(
        Admin::new(redirect)
            .model::<users::Entity>()
            .router(&ctx)
            .unwrap(),
    );
    assert_eq!(client.get("/api/me").await.status, 401);
    assert_eq!(
        client.send(Method::POST, "/api/session", None).await.status,
        404,
        "no sign-in without the login form"
    );
    assert_eq!(
        client
            .send(Method::DELETE, "/api/session", None)
            .await
            .status,
        204
    );
    assert!(
        Admin::new(JwtAuth::<users::Model>::new(|_| true).login_method(LoginMethod::None))
            .model::<users::Entity>()
            .router(&ctx)
            .is_ok()
    );
}

#[tokio::test]
async fn sessions_signed_with_the_development_secret_are_ignored_elsewhere() {
    let test_db = database().await;
    let staff = create_user(&test_db.db, "staff@example.com", true).await;
    // A cookie signed with Loco's public development secret, as anyone could forge.
    let dev = mount(&test_db, Environment::Development, json!({})).unwrap();
    let forged = sign_in(&dev, &staff).await;

    let ctx = context(
        &test_db,
        Environment::Any("staging".into()),
        &config(json!({}), None),
    );
    let redirect = JwtAuth::<users::Model>::new(|_| true).login_method(LoginMethod::Redirect {
        url: "/login".into(),
    });
    let staging = Client::new(
        Admin::new(redirect)
            .model::<users::Entity>()
            .router(&ctx)
            .unwrap(),
    );
    assert_eq!(
        send(&staging, Method::GET, "/api/me", &forged).await.status,
        401
    );
}

#[tokio::test]
async fn bearer_tokens_still_work_without_a_session() {
    let (db, ctx) = setup().await;
    let client = Client::new(admin().router(&ctx).unwrap());
    let staff = create_user(&db.db, "staff@example.com", true).await;
    let res = client
        .with_token(token(&staff.pid.to_string()))
        .get("/api/me")
        .await;
    assert_eq!(res.status, 200);
    assert!(res.header("set-cookie").is_none(), "scripts get no cookie");
}

use dogfood::{
    app::App,
    models::{
        _entities::{article_tags, articles},
        articles::ArticleStatus,
    },
};
use loco_rs::{testing::prelude::*, TestServer};
use sea_orm::{EntityTrait, PaginatorTrait};
use serde_json::{json, Value};
use serial_test::serial;

const ADMIN: (&str, &str) = ("user1@example.com", "12341234");
const NON_ADMIN: (&str, &str) = ("user2@example.com", "12341234");
const EDITOR: (&str, &str) = ("editor@example.com", "12341234");
const CSRF: (&str, &str) = ("x-requested-with", "muy-loco-admin");

async fn login(request: &TestServer, (email, password): (&str, &str)) -> String {
    let res = request
        .post("/api/auth/login")
        .json(&json!({"email": email, "password": password}))
        .await;
    assert_eq!(res.status_code(), 200, "{}", res.text());
    res.json::<Value>()["token"].as_str().unwrap().to_string()
}

#[tokio::test]
#[serial]
async fn serves_the_ui_and_requires_admin_users() {
    request::<App, _, _>(|request, ctx| async move {
        seed::<App>(&ctx).await.unwrap();

        let res = request.get("/admin").await;
        assert_eq!(res.status_code(), 200);
        assert!(
            res.headers().contains_key("x-request-id"),
            "the app's middleware wraps the admin"
        );
        assert!(res
            .text()
            .contains(r#"<meta name="mla-base" content="/admin" />"#));

        let config = request.get("/admin/api/config").await.json::<Value>();
        assert_eq!(config["title"], "Muy Loco Admin");
        assert_eq!(config["login"]["endpoint"], "/api/auth/login");

        assert_eq!(request.get("/admin/api/me").await.status_code(), 401);

        let token = login(&request, NON_ADMIN).await;
        let res = request
            .get("/admin/api/me")
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.status_code(), 403);

        let token = login(&request, ADMIN).await;
        let res = request
            .get("/admin/api/me")
            .authorization_bearer(&token)
            .await;
        assert_eq!(res.status_code(), 200);
        assert_eq!(res.json::<Value>()["name"], "user1@example.com");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn exposes_the_schema_without_secrets() {
    request::<App, _, _>(|request, ctx| async move {
        seed::<App>(&ctx).await.unwrap();
        let token = login(&request, ADMIN).await;
        let schema = request
            .get("/admin/api/schema")
            .authorization_bearer(&token)
            .await
            .json::<Value>();
        let models: Vec<&str> = schema["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            models,
            ["users", "articles", "categories", "tags", "article_tags"]
        );

        let users = &schema["models"][0];
        let fields: Vec<&str> = users["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        for secret in [
            "password",
            "api_key",
            "reset_token",
            "email_verification_token",
            "magic_link_token",
        ] {
            assert!(!fields.contains(&secret), "{secret} must be hidden");
        }
        assert_eq!(users["capabilities"]["create"], false);

        let articles = &schema["models"][1];
        let status = articles["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "status")
            .unwrap();
        assert_eq!(status["choices"], json!(["draft", "review", "published"]));
        assert_eq!(articles["primary_key"], json!(["id"]));
        assert_eq!(
            schema["models"][4]["primary_key"],
            json!(["article_id", "tag_id"])
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn lists_creates_updates_and_deletes() {
    request::<App, _, _>(|request, ctx| async move {
        seed::<App>(&ctx).await.unwrap();
        let token = login(&request, ADMIN).await;

        let list = request
            .get("/admin/api/models/articles/records?filter.status=published&sort=views&order=desc")
            .authorization_bearer(&token)
            .await
            .json::<Value>();
        assert_eq!(list["records"].as_array().unwrap().len(), 5);
        let count = request
            .get("/admin/api/models/articles/count?filter.status=published")
            .authorization_bearer(&token)
            .await
            .json::<Value>();
        assert_eq!(count["count"], json!({"kind": "exact", "value": 5}));
        assert_eq!(
            list["records"][0]["values"]["title"],
            "100% uptime is a myth"
        );
        assert_eq!(list["labels"]["user_id"]["1"], "user1@example.com");

        let created = request
            .post("/admin/api/models/categories/records")
            .authorization_bearer(&token)
            .add_header(CSRF.0, CSRF.1)
            .json(&json!({"name": "Ops", "slug": "ops"}))
            .await;
        assert_eq!(created.status_code(), 201, "{}", created.text());
        let key = created.json::<Value>()["record"]["key"]
            .as_str()
            .unwrap()
            .to_string();

        let duplicate = request
            .post("/admin/api/models/categories/records")
            .authorization_bearer(&token)
            .add_header(CSRF.0, CSRF.1)
            .json(&json!({"name": "Ops again", "slug": "ops"}))
            .await;
        assert_eq!(duplicate.status_code(), 409);

        let invalid = request
            .patch("/admin/api/models/articles/records/1")
            .authorization_bearer(&token)
            .add_header(CSRF.0, CSRF.1)
            .json(&json!({"title": "", "rating": 9}))
            .await;
        assert_eq!(invalid.status_code(), 422);
        let fields = &invalid.json::<Value>()["fields"];
        assert_eq!(fields["title"], json!(["title must not be empty"]));
        assert_eq!(fields["rating"], json!(["rating must be between 0 and 5"]));

        let updated = request
            .patch("/admin/api/models/articles/records/1")
            .authorization_bearer(&token)
            .add_header(CSRF.0, CSRF.1)
            .json(&json!({"title": "Renamed", "category_id": key, "status": "review"}))
            .await;
        assert_eq!(updated.status_code(), 200, "{}", updated.text());
        let body = updated.json::<Value>();
        assert_eq!(body["record"]["values"]["status"], "review");
        assert_eq!(body["labels"]["category_id"][&key], "Ops");

        let before = article_tags::Entity::find().count(&ctx.db).await.unwrap();
        let deleted = request
            .post("/admin/api/models/articles/bulk-delete")
            .authorization_bearer(&token)
            .add_header(CSRF.0, CSRF.1)
            .json(&json!({"keys": ["1", "2"]}))
            .await;
        assert_eq!(deleted.json::<Value>()["deleted"], 2);
        let after = article_tags::Entity::find().count(&ctx.db).await.unwrap();
        assert_eq!(before - after, 4, "join rows cascade");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn runs_actions_and_exports() {
    request::<App, _, _>(|request, ctx| async move {
        seed::<App>(&ctx).await.unwrap();
        let token = login(&request, ADMIN).await;

        let res = request
            .post("/admin/api/models/articles/actions/publish")
            .authorization_bearer(&token)
            .add_header(CSRF.0, CSRF.1)
            .json(&json!({"keys": ["4", "6"]}))
            .await;
        assert_eq!(res.status_code(), 200, "{}", res.text());
        assert_eq!(res.json::<Value>()["message"], "Published 2 articles");
        let article = articles::Entity::find_by_id(4)
            .one(&ctx.db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(article.status, ArticleStatus::Published);
        assert!(article.published_at.is_some());

        let csv = request
            .get("/admin/api/models/articles/export?filter.status=draft")
            .authorization_bearer(&token)
            .await;
        assert_eq!(csv.status_code(), 200);
        let text = csv.text();
        assert!(
            text.starts_with("id,title,")
                && text
                    .lines()
                    .next()
                    .unwrap()
                    .ends_with(",created_at,updated_at"),
            "key first, timestamps last: {text}"
        );
        assert_eq!(text.lines().count(), 1 + 2);

        let tag = request
            .get("/admin/api/models/article_tags/records/1%2C2")
            .authorization_bearer(&token)
            .await;
        assert_eq!(tag.status_code(), 200, "{}", tag.text());
        assert_eq!(tag.json::<Value>()["labels"]["tag_id"]["2"], "web");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn roles_limit_admin_users() {
    request::<App, _, _>(|request, ctx| async move {
        seed::<App>(&ctx).await.unwrap();
        let admin = login(&request, ADMIN).await;
        let roles = request
            .get("/admin/api/roles")
            .authorization_bearer(&admin)
            .await
            .json::<Value>();
        assert_eq!(roles["roles"][0]["name"], "Editor");
        assert_eq!(roles["roles"][0]["users"], 1);

        let editor = login(&request, EDITOR).await;
        let me = request
            .get("/admin/api/me")
            .authorization_bearer(&editor)
            .await
            .json::<Value>();
        assert_eq!(me["role"]["name"], "Editor");
        assert_eq!(me["can_manage_roles"], false);
        let schema = request
            .get("/admin/api/schema")
            .authorization_bearer(&editor)
            .await
            .json::<Value>();
        let models: Vec<&str> = schema["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        assert_eq!(models, ["articles", "categories", "tags", "article_tags"]);
        let articles = &schema["models"][0];
        assert_eq!(
            articles["capabilities"],
            json!({"view": true, "create": true, "update": true, "delete": false, "export": false})
        );
        let author = articles["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "user_id")
            .unwrap();
        assert_eq!(author["relation"]["model"], Value::Null, "a plain id field");

        let users = request
            .get("/admin/api/models/users/records")
            .authorization_bearer(&editor)
            .await;
        assert_eq!(users.status_code(), 403);
        let deleted = request
            .delete("/admin/api/models/articles/records/1")
            .authorization_bearer(&editor)
            .add_header(CSRF.0, CSRF.1)
            .await;
        assert_eq!(deleted.status_code(), 403);
        let roles = request
            .get("/admin/api/roles")
            .authorization_bearer(&editor)
            .await;
        assert_eq!(roles.status_code(), 403);

        // The role cannot be deleted while the editor has it.
        let res = request
            .delete("/admin/api/roles/1")
            .authorization_bearer(&admin)
            .add_header(CSRF.0, CSRF.1)
            .await;
        assert_eq!(res.status_code(), 409);
    })
    .await;
}

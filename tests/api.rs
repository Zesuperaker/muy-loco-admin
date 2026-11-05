mod common;

use axum::http::{Method, StatusCode};
use common::{
    Client, TestDb, count, create_post, create_user, notes, post_tags, posts, setup, users,
};
use loco_rs::app::AppContext;
use muy_loco_admin::{Admin, AllowAll, ModelAdmin, SortDirection};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
use serde_json::{Value, json};

fn admin() -> Admin {
    Admin::new(AllowAll)
        .title("Test Admin")
        .register(ModelAdmin::<users::Entity>::new().can_create(false))
        .register(
            ModelAdmin::<posts::Entity>::new()
                .action("publish", "Publish", |ctx: AppContext, posts| async move {
                    let n = posts.len();
                    for post in posts {
                        let mut am: posts::ActiveModel = post.into_active_model();
                        am.status = Set(posts::PostStatus::Published);
                        am.update(&ctx.db).await?;
                    }
                    Ok(format!("published {n} posts"))
                })
                .action("fail", "Always fails", |_, _| async {
                    Err(loco_rs::Error::string("the action failed"))
                }),
        )
        .model::<post_tags::Entity>()
        .register(
            ModelAdmin::<notes::Entity>::new()
                .label("Notebook")
                .ordering("content", SortDirection::Asc)
                .per_page(2)
                .can_export(false),
        )
}

async fn client() -> (TestDb, AppContext, Client) {
    let (db, ctx) = setup().await;
    let router = admin().router(&ctx).expect("router");
    (db, ctx, Client::new(router))
}

fn model<'a>(schema: &'a Value, name: &str) -> &'a Value {
    schema["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("model {name} not in schema"))
}

fn field<'a>(model: &'a Value, name: &str) -> Option<&'a Value> {
    model["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
}

fn new_post(user_id: i32) -> Value {
    json!({
        "title": "Hello admin",
        "body": "Some body",
        "status": "draft",
        "featured": "on",
        "rating": "4.5",
        "price": "12.50",
        "views": "7",
        "published_on": "2024-05-01",
        "meta": {"tags": ["a", "b"]},
        "user_id": user_id.to_string(),
    })
}

#[tokio::test]
async fn config_is_public() {
    let (_db, _ctx, client) = client().await;
    let res = client.get("/api/config").await;
    assert_eq!(res.status, 200);
    let body = res.json();
    assert_eq!(body["title"], "Test Admin");
    assert_eq!(body["base_path"], "/admin");
    assert_eq!(body["login"]["type"], "none");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn me_returns_the_user() {
    let (_db, _ctx, client) = client().await;
    let body = client.get("/api/me").await.json();
    assert_eq!(body, json!({"id": "anonymous", "name": "Anonymous"}));
}

#[tokio::test]
async fn schema_describes_models() {
    let (_db, _ctx, client) = client().await;
    let res = client.get("/api/schema").await;
    assert_eq!(res.status, 200);
    let schema = res.json();
    let names: Vec<&str> = schema["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["users", "posts", "post_tags", "notes"]);

    let users = model(&schema, "users");
    assert!(
        field(users, "password").is_none(),
        "password must be hidden"
    );
    assert!(field(users, "api_key").is_none(), "api_key must be hidden");
    assert_eq!(users["display_field"], "name");
    assert_eq!(users["capabilities"]["create"], false);
    assert_eq!(
        users["related"],
        json!([{"model": "posts", "field": "user_id", "label": "Posts"}])
    );
    let id = field(users, "id").unwrap();
    assert_eq!(id["primary_key"], true);
    assert_eq!(id["auto_increment"], true);
    assert_eq!(id["readonly"], true);
    assert_eq!(field(users, "created_at").unwrap()["readonly"], true);
    assert_eq!(field(users, "created_at").unwrap()["timezone"], true);
    assert_eq!(field(users, "email").unwrap()["unique"], true);

    let posts = model(&schema, "posts");
    assert_eq!(posts["label"], "Posts");
    assert_eq!(posts["display_field"], "title");
    assert_eq!(
        posts["ordering"],
        json!({"field": "id", "direction": "desc"})
    );
    assert_eq!(posts["per_page"], 25);
    let status = field(posts, "status").unwrap();
    assert_eq!(status["kind"], "enum");
    assert_eq!(status["choices"], json!(["draft", "published"]));
    let user_id = field(posts, "user_id").unwrap();
    assert_eq!(user_id["label"], "User");
    assert_eq!(
        user_id["relation"],
        json!({"table": "users", "column": "id", "model": "users"})
    );
    assert_eq!(field(posts, "body").unwrap()["kind"], "text");
    assert_eq!(field(posts, "price").unwrap()["kind"], "decimal");
    assert_eq!(field(posts, "meta").unwrap()["kind"], "json");
    assert_eq!(field(posts, "rating").unwrap()["nullable"], true);
    assert!(
        !posts["list_display"]
            .as_array()
            .unwrap()
            .contains(&json!("body"))
    );
    assert!(
        !posts["list_display"]
            .as_array()
            .unwrap()
            .contains(&json!("meta"))
    );
    assert_eq!(posts["search_fields"], json!(["title", "body"]));
    assert_eq!(
        posts["list_filter"],
        json!(["status", "featured", "user_id"])
    );
    assert_eq!(
        posts["actions"],
        json!([{"name": "publish", "label": "Publish"}, {"name": "fail", "label": "Always fails"}])
    );
    assert_eq!(
        posts["related"],
        json!([{"model": "post_tags", "field": "post_id", "label": "Post tags"}])
    );

    let tags = model(&schema, "post_tags");
    assert_eq!(tags["primary_key"], json!(["post_id", "tag"]));
    assert_eq!(field(tags, "tag").unwrap()["readonly"], false);
    assert_eq!(field(tags, "weight").unwrap()["kind"], "integer");

    let notes = model(&schema, "notes");
    assert_eq!(notes["label"], "Notebook");
    assert_eq!(notes["per_page"], 2);
    assert_eq!(notes["capabilities"]["export"], false);
    assert!(field(notes, "secret_token").is_none());
    assert_eq!(field(notes, "id").unwrap()["kind"], "uuid");
    assert_eq!(field(notes, "id").unwrap()["auto_increment"], false);
    assert_eq!(field(notes, "reminder_at").unwrap()["timezone"], false);
    assert_eq!(field(notes, "remind_time").unwrap()["kind"], "time");
    assert_eq!(field(notes, "attachment").unwrap()["kind"], "binary");
    assert_eq!(field(notes, "attachment").unwrap()["readonly"], true);
}

#[tokio::test]
async fn create_show_and_typed_values() {
    let (db, _ctx, client) = client().await;
    let user = create_user(&db.db, "ann@example.com", true).await;

    let res = client
        .post("/api/models/posts/records", new_post(user.id))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.text());
    let body = res.json();
    let record = &body["record"];
    let values = &record["values"];
    assert_eq!(values["title"], "Hello admin");
    assert_eq!(values["status"], "draft");
    assert_eq!(values["featured"], true);
    assert_eq!(values["rating"], 4.5);
    let price: f64 = values["price"].as_str().unwrap().parse().unwrap();
    assert!((price - 12.5).abs() < f64::EPSILON, "decimals are strings");
    assert_eq!(values["views"], 7);
    assert_eq!(values["published_on"], "2024-05-01");
    assert_eq!(values["meta"], json!({"tags": ["a", "b"]}));
    assert!(values["created_at"].is_string(), "hook sets created_at");
    assert_eq!(record["label"], "Hello admin");
    assert_eq!(body["labels"]["user_id"][user.id.to_string()], "ann");

    let key = record["key"].as_str().unwrap();
    let show = client
        .get(&format!("/api/models/posts/records/{key}"))
        .await;
    assert_eq!(show.status, 200);
    assert_eq!(show.json()["record"], body["record"]);
}

#[tokio::test]
async fn create_reports_field_errors() {
    let (db, _ctx, client) = client().await;
    let user = create_user(&db.db, "ann@example.com", true).await;

    let mut input = new_post(user.id);
    input["views"] = json!("many");
    input["status"] = json!("archived");
    input["id"] = json!(5);
    input["created_at"] = json!("2024-01-01T00:00:00Z");
    input["nope"] = json!(1);
    let res = client.post("/api/models/posts/records", input).await;
    assert_eq!(res.status, 422);
    let fields = &res.json()["fields"];
    assert_eq!(fields["views"], json!(["must be an integer"]));
    assert_eq!(
        fields["status"],
        json!(["must be one of: draft, published"])
    );
    assert_eq!(fields["id"], json!(["is read-only"]));
    assert_eq!(fields["created_at"], json!(["is read-only"]));
    assert_eq!(fields["nope"], json!(["is not a field of this model"]));

    let res = client
        .post(
            "/api/models/users/records",
            json!({"email": "x@example.com"}),
        )
        .await;
    assert_eq!(res.status, 403, "users cannot be created");

    let res = client
        .post("/api/models/notes/records", json!({"id": "00000000-0000-0000-0000-000000000001", "content": "x", "secret_token": "s"}))
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(
        res.json()["fields"]["secret_token"],
        json!(["is not editable"])
    );

    assert_eq!(count::<posts::Entity>(&db.db).await, 0);
}

#[tokio::test]
async fn create_surfaces_hook_and_database_errors() {
    let (db, _ctx, client) = client().await;
    let user = create_user(&db.db, "ann@example.com", true).await;

    let mut input = new_post(user.id);
    input["title"] = json!("  ");
    let res = client.post("/api/models/posts/records", input).await;
    assert_eq!(res.status, 422);
    let body = res.json();
    assert_eq!(body["error"], "validation failed");
    assert_eq!(body["fields"]["title"], json!(["title must not be empty"]));

    let mut input = new_post(user.id);
    input["title"] = json!("hook-error");
    let res = client.post("/api/models/posts/records", input).await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json()["error"], "rejected by hook");

    let mut input = new_post(user.id);
    input.as_object_mut().unwrap().remove("body");
    let res = client.post("/api/models/posts/records", input).await;
    assert_eq!(res.status, 422, "missing NOT NULL column: {}", res.text());

    let res = client
        .send_raw(
            Method::POST,
            "/api/models/posts/records",
            Some("{oops".into()),
            vec![],
        )
        .await;
    assert_eq!(res.status, 400);
    let res = client
        .send_raw(
            Method::POST,
            "/api/models/posts/records",
            Some("[1]".into()),
            vec![],
        )
        .await;
    assert_eq!(res.status, 400);
}

#[tokio::test]
async fn unique_violations_are_conflicts() {
    let (db, _ctx, client) = client().await;
    create_user(&db.db, "ann@example.com", true).await;
    let bob = create_user(&db.db, "bob@example.com", false).await;
    let res = client
        .patch(
            &format!("/api/models/users/records/{}", bob.id),
            json!({"email": "ann@example.com"}),
        )
        .await;
    assert_eq!(res.status, 409, "{}", res.text());
}

#[tokio::test]
async fn list_paginates_sorts_and_labels() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    for i in 1..=7 {
        create_post(&db.db, ann.id, &format!("Post {i}")).await;
    }

    let res = client
        .get("/api/models/posts/records?per_page=3&page=2")
        .await;
    assert_eq!(res.status, 200);
    let body = res.json();
    assert_eq!(body["total"], 7);
    assert_eq!(body["pages"], 3);
    assert_eq!(body["page"], 2);
    assert_eq!(body["per_page"], 3);
    let titles: Vec<&str> = body["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["values"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        vec!["Post 4", "Post 3", "Post 2"],
        "default order is id desc"
    );
    assert_eq!(body["labels"]["user_id"][ann.id.to_string()], "ann");

    let body = client
        .get("/api/models/posts/records?sort=title&order=asc&per_page=2")
        .await
        .json();
    assert_eq!(body["records"][0]["values"]["title"], "Post 1");
    let body = client
        .get("/api/models/posts/records?sort=title&per_page=1")
        .await
        .json();
    assert_eq!(
        body["records"][0]["values"]["title"], "Post 1",
        "explicit sort defaults to asc"
    );
    let body = client
        .get("/api/models/posts/records?sort=title&order=desc&per_page=1")
        .await
        .json();
    assert_eq!(body["records"][0]["values"]["title"], "Post 7");

    let res = client.get("/api/models/posts/records?sort=password").await;
    assert_eq!(res.status, 400);
    let res = client.get("/api/models/posts/records?page=abc").await;
    assert_eq!(res.status, 400);
    let res = client.get("/api/models/nothing/records").await;
    assert_eq!(res.status, 404);

    let body = client.get("/api/models/notes/records").await.json();
    assert_eq!(body["per_page"], 2, "model default page size");
}

#[tokio::test]
async fn list_search_and_filters() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let bob = create_user(&db.db, "bob@example.com", false).await;
    let a = create_post(&db.db, ann.id, "Rust tips").await;
    create_post(&db.db, ann.id, "100% pure").await;
    create_post(&db.db, bob.id, "Gardening").await;
    let mut am: posts::ActiveModel = a.into_active_model();
    am.featured = Set(true);
    am.status = Set(posts::PostStatus::Published);
    am.rating = Set(Some(5.0));
    am.update(&db.db).await.unwrap();

    let titles = |body: Value| -> Vec<String> {
        body["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["values"]["title"].as_str().unwrap().to_string())
            .collect()
    };

    let body = client.get("/api/models/posts/records?q=RUST").await.json();
    assert_eq!(
        titles(body),
        vec!["Rust tips"],
        "search is case-insensitive"
    );
    let body = client.get("/api/models/posts/records?q=%25").await.json();
    assert_eq!(
        titles(body),
        vec!["100% pure"],
        "LIKE wildcards are escaped"
    );
    let body = client
        .get("/api/models/posts/records?q=body%20of%20garden")
        .await
        .json();
    assert_eq!(titles(body), vec!["Gardening"], "body is searched too");

    let body = client
        .get("/api/models/posts/records?filter.featured=true")
        .await
        .json();
    assert_eq!(titles(body), vec!["Rust tips"]);
    let body = client
        .get("/api/models/posts/records?filter.status=published")
        .await
        .json();
    assert_eq!(titles(body), vec!["Rust tips"]);
    let body = client
        .get(&format!(
            "/api/models/posts/records?filter.user_id={}",
            bob.id
        ))
        .await
        .json();
    assert_eq!(titles(body), vec!["Gardening"]);
    let body = client
        .get("/api/models/posts/records?filter.rating=__notnull__")
        .await
        .json();
    assert_eq!(titles(body), vec!["Rust tips"]);
    let body = client
        .get("/api/models/posts/records?filter.rating=__null__&sort=title")
        .await
        .json();
    assert_eq!(titles(body), vec!["100% pure", "Gardening"]);

    let res = client
        .get("/api/models/posts/records?filter.views=lots")
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json()["fields"]["views"], json!(["must be an integer"]));
    let res = client
        .get("/api/models/posts/records?filter.status=gone")
        .await;
    assert_eq!(res.status, 422);
    let res = client
        .get("/api/models/posts/records?filter.password=x")
        .await;
    assert_eq!(res.status, 400);

    // Users are searchable through their text columns; `password` is not.
    let body = client
        .get("/api/models/users/records?q=hashed")
        .await
        .json();
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn show_handles_missing_and_malformed_keys() {
    let (_db, _ctx, client) = client().await;
    assert_eq!(
        client.get("/api/models/posts/records/999").await.status,
        404
    );
    assert_eq!(
        client.get("/api/models/posts/records/abc").await.status,
        404
    );
    assert_eq!(
        client.get("/api/models/post_tags/records/1").await.status,
        404
    );
    assert_eq!(
        client.get("/api/models/missing/records/1").await.status,
        404
    );
}

#[tokio::test]
async fn update_runs_hooks_and_validates() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let post = create_post(&db.db, ann.id, "Original").await;
    let uri = format!("/api/models/posts/records/{}", post.id);

    let res = client
        .patch(
            &uri,
            json!({"title": "Changed", "rating": 3, "published_on": "", "meta": null}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    let values = &res.json()["record"]["values"];
    assert_eq!(values["title"], "Changed");
    assert_eq!(values["rating"], 3.0);
    assert_eq!(values["published_on"], Value::Null);
    assert_ne!(
        values["updated_at"],
        json!(post.updated_at.to_rfc3339()),
        "before_save hook bumps updated_at"
    );

    let unchanged = client.patch(&uri, json!({"title": "Changed"})).await;
    assert_eq!(unchanged.status, 200);
    assert_eq!(
        unchanged.json()["record"]["values"]["updated_at"],
        values["updated_at"]
    );

    let res = client.patch(&uri, json!({"title": ""})).await;
    assert_eq!(res.status, 422);
    assert_eq!(
        res.json()["fields"]["title"],
        json!(["title must not be empty"])
    );

    let res = client.patch(&uri, json!({"views": null, "id": 3})).await;
    assert_eq!(res.status, 422);
    let fields = &res.json()["fields"];
    assert_eq!(fields["views"], json!(["cannot be empty"]));
    assert_eq!(fields["id"], json!(["is read-only"]));

    let res = client
        .send(
            Method::PUT,
            &uri,
            Some(json!({"featured": false, "status": "published"})),
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["record"]["values"]["status"], "published");

    assert_eq!(
        client
            .patch("/api/models/posts/records/404", json!({}))
            .await
            .status,
        404
    );
    let stored = posts::Entity::find_by_id(post.id)
        .one(&db.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.title, "Changed");
    assert_eq!(stored.status, posts::PostStatus::Published);
}

#[tokio::test]
async fn delete_runs_hooks_in_a_transaction() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let a = create_post(&db.db, ann.id, "first").await;
    let locked = create_post(&db.db, ann.id, "locked").await;
    let c = create_post(&db.db, ann.id, "third").await;

    let res = client
        .delete(&format!("/api/models/posts/records/{}", a.id))
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json(), json!({"deleted": 1}));
    assert_eq!(count::<posts::Entity>(&db.db).await, 2);

    let res = client
        .delete(&format!("/api/models/posts/records/{}", locked.id))
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json()["error"], "locked posts cannot be deleted");

    let res = client
        .post(
            "/api/models/posts/bulk-delete",
            json!({"keys": [c.id.to_string(), locked.id.to_string()]}),
        )
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(
        count::<posts::Entity>(&db.db).await,
        2,
        "bulk delete rolled back"
    );

    let res = client
        .post(
            "/api/models/posts/bulk-delete",
            json!({"keys": [c.id.to_string()]}),
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json(), json!({"deleted": 1}));

    let res = client
        .post("/api/models/posts/bulk-delete", json!({"keys": []}))
        .await;
    assert_eq!(res.status, 400);
    let res = client
        .post("/api/models/posts/bulk-delete", json!({"ids": []}))
        .await;
    assert_eq!(res.status, 400);
    let res = client
        .post("/api/models/posts/bulk-delete", json!({"keys": ["999"]}))
        .await;
    assert_eq!(res.status, 404);
}

#[tokio::test]
async fn export_writes_csv() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    create_post(&db.db, ann.id, "=HYPERLINK(\"evil\")").await;
    create_post(&db.db, ann.id, "plain, with comma").await;

    let res = client
        .get("/api/models/posts/export?sort=id&order=asc")
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.header("content-type"), Some("text/csv; charset=utf-8"));
    assert_eq!(
        res.header("content-disposition"),
        Some("attachment; filename=\"posts.csv\"")
    );
    let text = res.text();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("id,title,body,status,featured,rating,price,views"));
    assert!(
        lines[1].contains("\"'=HYPERLINK(\"\"evil\"\")\""),
        "{}",
        lines[1]
    );
    assert!(lines[2].contains("\"plain, with comma\""));
    assert_eq!(lines.len(), 3);

    let text = client.get("/api/models/posts/export?q=plain").await.text();
    assert_eq!(text.lines().count(), 2);

    assert_eq!(client.get("/api/models/notes/export").await.status, 403);
    assert_eq!(
        client
            .get("/api/models/posts/export?filter.views=x")
            .await
            .status,
        422
    );
}

#[tokio::test]
async fn lookup_returns_choices() {
    let (db, _ctx, client) = client().await;
    for name in ["ann", "anna", "bob"] {
        create_user(&db.db, &format!("{name}@example.com"), false).await;
    }
    let body = client.get("/api/models/users/lookup?q=ann").await.json();
    let labels: Vec<&str> = body["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["anna", "ann"]);
    assert!(body["choices"][0]["value"].is_number());

    let body = client
        .get("/api/models/users/lookup?column=email&limit=1")
        .await
        .json();
    assert_eq!(body["choices"].as_array().unwrap().len(), 1);
    assert_eq!(body["choices"][0]["value"], "bob@example.com");

    let res = client.get("/api/models/users/lookup?column=password").await;
    assert_eq!(res.status, 400);
}

#[tokio::test]
async fn actions_run_on_selected_records() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let a = create_post(&db.db, ann.id, "a").await;
    let b = create_post(&db.db, ann.id, "b").await;

    let res = client
        .post(
            "/api/models/posts/actions/publish",
            json!({"keys": [a.id.to_string(), b.id.to_string()]}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(res.json(), json!({"message": "published 2 posts"}));
    let stored = posts::Entity::find_by_id(a.id)
        .one(&db.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, posts::PostStatus::Published);

    let res = client
        .post(
            "/api/models/posts/actions/fail",
            json!({"keys": [a.id.to_string()]}),
        )
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json()["error"], "the action failed");

    let res = client
        .post("/api/models/posts/actions/unknown", json!({"keys": ["1"]}))
        .await;
    assert_eq!(res.status, 404);
    let res = client
        .post(
            "/api/models/posts/actions/publish",
            json!({"keys": ["999"]}),
        )
        .await;
    assert_eq!(res.status, 404);
}

#[tokio::test]
async fn composite_keys() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let post = create_post(&db.db, ann.id, "tagged").await;

    let res = client
        .post(
            "/api/models/post_tags/records",
            json!({"post_id": post.id, "tag": "a,b%", "weight": 2}),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.text());
    let key = res.json()["record"]["key"].as_str().unwrap().to_string();
    assert_eq!(key, format!("{},a%2Cb%25", post.id));
    assert_eq!(
        res.json()["labels"]["post_id"][post.id.to_string()],
        "tagged"
    );

    let encoded = key.replace('%', "%25").replace(',', "%2C");
    let uri = format!("/api/models/post_tags/records/{encoded}");
    let res = client.get(&uri).await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(res.json()["record"]["values"]["tag"], "a,b%");

    let res = client.patch(&uri, json!({"weight": 5})).await;
    assert_eq!(res.json()["record"]["values"]["weight"], 5);
    let res = client.patch(&uri, json!({"tag": "x"})).await;
    assert_eq!(res.status, 422, "primary keys are read-only on update");

    assert_eq!(client.delete(&uri).await.status, 200);
    assert_eq!(count::<post_tags::Entity>(&db.db).await, 0);
}

#[tokio::test]
async fn uuid_keys_and_naive_temporal_values() {
    let (db, _ctx, client) = client().await;
    let id = "67e55044-10b1-426f-9247-bb680e5fe0c8";
    let res = client
        .post(
            "/api/models/notes/records",
            json!({
                "id": id,
                "content": "remember",
                "reminder_at": "2024-03-01T09:30",
                "remind_time": "09:30",
                "pinned": "",
            }),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.text());
    let record = &res.json()["record"];
    assert_eq!(record["key"], id);
    assert_eq!(record["label"], format!("#{id}"));
    assert_eq!(record["values"]["reminder_at"], "2024-03-01T09:30:00");
    assert_eq!(record["values"]["remind_time"], "09:30:00");
    assert_eq!(record["values"]["pinned"], Value::Null);
    assert_eq!(record["values"]["attachment"], Value::Null);

    let res = client
        .post(
            "/api/models/notes/records",
            json!({"content": "missing id"}),
        )
        .await;
    assert!(res.status.is_client_error() || res.status.is_server_error());
    assert_eq!(count::<notes::Entity>(&db.db).await, 1);

    let res = client.get(&format!("/api/models/notes/records/{id}")).await;
    assert_eq!(res.status, 200);
    assert_eq!(
        client
            .get("/api/models/notes/records/not-a-uuid")
            .await
            .status,
        404
    );
}

#[tokio::test]
async fn stats_count_records() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    create_post(&db.db, ann.id, "x").await;
    let body = client.get("/api/stats").await.json();
    assert_eq!(
        body["counts"],
        json!({"users": 1, "posts": 1, "post_tags": 0, "notes": 0})
    );
}

#[tokio::test]
async fn mutations_require_the_csrf_header() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let client = client.without_csrf();
    let res = client
        .post("/api/models/posts/records", new_post(ann.id))
        .await;
    assert_eq!(res.status, 400);
    assert!(
        res.json()["error"]
            .as_str()
            .unwrap()
            .contains("x-requested-with")
    );
    assert_eq!(
        client.get("/api/schema").await.status,
        200,
        "reads need no header"
    );
}

#[tokio::test]
async fn unknown_api_routes_are_json_404() {
    let (_db, _ctx, client) = client().await;
    let res = client.get("/api/nothing").await;
    assert_eq!(res.status, 404);
    assert_eq!(res.json(), json!({"error": "not found"}));
}

#[tokio::test]
async fn invalid_model_configuration_fails_at_mount() {
    let (_db, ctx) = setup().await;
    let cases: Vec<(Admin, &str)> = vec![
        (
            Admin::new(AllowAll)
                .register(ModelAdmin::<posts::Entity>::new().list_display(["nope"])),
            "unknown field `nope` in list_display",
        ),
        (
            Admin::new(AllowAll)
                .register(ModelAdmin::<users::Entity>::new().search_fields(["password"])),
            "hidden field `password` used in search_fields",
        ),
        (
            Admin::new(AllowAll).register(ModelAdmin::<users::Entity>::new().hidden_fields(["id"])),
            "primary key `id` cannot be hidden",
        ),
        (
            Admin::new(AllowAll).register(ModelAdmin::<users::Entity>::new().hidden_fields(["x"])),
            "unknown field `x` in hidden_fields",
        ),
        (
            Admin::new(AllowAll)
                .register(ModelAdmin::<users::Entity>::new().readonly_fields(["x"])),
            "unknown field `x` in readonly_fields",
        ),
        (
            Admin::new(AllowAll).register(ModelAdmin::<users::Entity>::new().list_filter(["x"])),
            "unknown field `x` in list_filter",
        ),
        (
            Admin::new(AllowAll).register(
                ModelAdmin::<users::Entity>::new().ordering("password", SortDirection::Asc),
            ),
            "hidden field `password` used in ordering",
        ),
        (
            Admin::new(AllowAll).register(ModelAdmin::<users::Entity>::new().display_field("x")),
            "unknown field `x` in display_field",
        ),
        (
            Admin::new(AllowAll).register(
                ModelAdmin::<users::Entity>::new()
                    .action("a", "A", |_, _| async { Ok(String::new()) })
                    .action("a", "B", |_, _| async { Ok(String::new()) }),
            ),
            "duplicate action `a`",
        ),
        (
            Admin::new(AllowAll)
                .model::<users::Entity>()
                .model::<users::Entity>(),
            "model `users` is registered more than once",
        ),
        (Admin::new(AllowAll).path("/"), "must start with `/`"),
    ];
    for (admin, expected) in cases {
        let err = admin.router(&ctx).err().expect("configuration error");
        assert!(
            err.to_string().contains(expected),
            "{err} should contain {expected}"
        );
    }
}

#[tokio::test]
async fn custom_configuration_is_applied() {
    let (db, ctx) = setup().await;
    let router = Admin::new(AllowAll)
        .register(
            ModelAdmin::<users::Entity>::new()
                .name("people")
                .hidden_fields(["password"])
                .readonly_fields(["email"])
                .list_display(["email", "api_key"])
                .search_fields(["api_key", "id"])
                .list_filter(["is_admin"])
                .display_field("email")
                .read_only()
                .can_export(true),
        )
        .router(&ctx)
        .unwrap();
    let client = Client::new(router);
    let ann = create_user(&db.db, "ann@example.com", true).await;

    let schema = client.get("/api/schema").await.json();
    let people = model(&schema, "people");
    assert_eq!(people["table"], "users");
    assert!(field(people, "api_key").is_some());
    assert_eq!(field(people, "email").unwrap()["readonly"], true);
    assert_eq!(people["list_display"], json!(["email", "api_key"]));
    assert_eq!(people["list_filter"], json!(["is_admin"]));
    assert_eq!(
        people["capabilities"],
        json!({"view": true, "create": false, "update": false, "delete": false, "export": true})
    );

    let body = client
        .get(&format!("/api/models/people/records?q={}", ann.id))
        .await
        .json();
    assert_eq!(body["total"], 1, "non-text search fields are cast to text");
    assert_eq!(body["records"][0]["label"], "ann@example.com");
    let res = client
        .patch(
            &format!("/api/models/people/records/{}", ann.id),
            json!({"name": "x"}),
        )
        .await;
    assert_eq!(res.status, 403);
    let res = client
        .delete(&format!("/api/models/people/records/{}", ann.id))
        .await;
    assert_eq!(res.status, 403);
    assert_eq!(client.get("/api/models/users/records").await.status, 404);
    drop(db);
}

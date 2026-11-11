mod common;

use axum::http::{Method, StatusCode};
use common::{
    Client, TestDb, count, create_post, create_user, exact, insert_posts, is_postgres, notes,
    post_tags, posts, setup, users,
};
use loco_rs::app::AppContext;
use muy_loco_admin::{Admin, AllowAll, MAX_PER_PAGE, ModelAdmin, SortDirection};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel, Set};
use serde_json::{Value, json};

fn admin() -> Admin {
    Admin::new(AllowAll)
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
                    Err(loco_rs::Error::string("db password is hunter2"))
                })
                .action("refuse", "Refuses", |_, _| async {
                    // A backtrace does not hide a message meant for the user.
                    Err(loco_rs::Error::WithBacktrace {
                        inner: Box::new(loco_rs::Error::BadRequest("nothing to do".into())),
                        backtrace: Box::new(std::backtrace::Backtrace::disabled()),
                    })
                })
                .action("retitle", "Retitle", |ctx: AppContext, posts| async move {
                    for post in posts {
                        let mut am: posts::ActiveModel = post.into_active_model();
                        am.title = Set("hook-error".into());
                        am.update(&ctx.db).await?;
                    }
                    Ok(String::new())
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
    let router = admin().router(&ctx).await.expect("router");
    (db, ctx, Client::new(router))
}

/// The admin with cursor pagination on the models whose lists tests page.
async fn cursor_client(ctx: &AppContext) -> Client {
    let router = Admin::new(AllowAll)
        .register(ModelAdmin::<users::Entity>::new().can_create(false))
        .register(ModelAdmin::<posts::Entity>::new().cursor_pagination())
        .register(ModelAdmin::<post_tags::Entity>::new().cursor_pagination())
        .register(ModelAdmin::<notes::Entity>::new().cursor_pagination())
        .router(ctx)
        .await
        .expect("router");
    Client::new(router)
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
    assert_eq!(body["title"], "Muy Loco Admin");
    assert_eq!(body["base_path"], "/admin");
    assert_eq!(body["site_url"], "/");
    assert_eq!(body["login"]["type"], "none");
    assert!(
        body.get("version").is_none(),
        "anyone can read it: no version"
    );
}

#[tokio::test]
async fn me_returns_the_user() {
    let (_db, _ctx, client) = client().await;
    let body = client.get("/api/me").await.json();
    assert_eq!(
        body,
        json!({"id": "anonymous", "name": "Anonymous", "role": null, "can_manage_roles": false})
    );
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
        json!([
            {"name": "publish", "label": "Publish"},
            {"name": "fail", "label": "Always fails"},
            {"name": "refuse", "label": "Refuses"},
            {"name": "retitle", "label": "Retitle"},
        ])
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
async fn fields_put_the_key_first_and_timestamps_last() {
    let (_db, _ctx, client) = client().await;
    let schema = client.get("/api/schema").await.json();
    let users = model(&schema, "users");
    let names: Vec<&str> = users["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    // The test entity declares its timestamps first, as Loco's generated entities do.
    assert_eq!(
        names,
        [
            "id",
            "pid",
            "email",
            "name",
            "is_admin",
            "admin_role_id",
            "created_at",
            "updated_at"
        ]
    );
    assert_eq!(users["list_display"], json!(names));
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
        json!(["is not a field of this model"]),
        "hidden fields are not revealed"
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
    assert_eq!(res.json()["error"], "the database rejected these values");

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
    assert_eq!(
        res.json(),
        json!({"error": "a record with the same value already exists"}),
        "no table or column names"
    );
}

#[tokio::test]
async fn list_paginates_sorts_and_labels() {
    let (db, _ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    for i in 1..=7 {
        create_post(&db.db, ann.id, &format!("Post {i}")).await;
    }

    let titles = |body: &Value| -> Vec<String> {
        body["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["values"]["title"].as_str().unwrap().to_string())
            .collect()
    };
    let schema = client.get("/api/schema").await.json();
    assert_eq!(model(&schema, "posts")["pagination"], "pages");

    // Numbered pages, by default.
    let first = client.get("/api/models/posts/records?per_page=3").await;
    assert_eq!(first.status, 200);
    let first = first.json();
    assert_eq!(
        titles(&first),
        ["Post 7", "Post 6", "Post 5"],
        "default order is id desc"
    );
    assert_eq!(first["per_page"], 3);
    assert_eq!(first["page"], 1);
    assert_eq!(first["prev"], Value::Null);
    assert_eq!(first["next"], "2");
    let body = client
        .get("/api/models/posts/records?per_page=3&page=2")
        .await
        .json();
    assert_eq!(titles(&body), ["Post 4", "Post 3", "Post 2"]);
    assert_eq!((&body["prev"], &body["next"]), (&json!("1"), &json!("3")));
    assert_eq!(body["labels"]["user_id"][ann.id.to_string()], "ann");
    let body = client
        .get("/api/models/posts/records?per_page=3&page=3")
        .await
        .json();
    assert_eq!(titles(&body), ["Post 1"]);
    assert_eq!(body["next"], Value::Null);
    let body = client
        .get("/api/models/posts/records?per_page=3&page=9")
        .await
        .json();
    assert_eq!(body["records"], json!([]), "past the last page");
    assert_eq!(
        client.get("/api/models/posts/count").await.json(),
        json!({"count": exact(7)})
    );

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
    let res = client
        .get("/api/models/posts/records?after=1&before=2")
        .await;
    assert_eq!(res.status, 400);
    let res = client.get("/api/models/posts/records?page=abc").await;
    assert_eq!(res.status, 400);
    let res = client.get("/api/models/nothing/records").await;
    assert_eq!(res.status, 404);

    let body = client.get("/api/models/notes/records").await.json();
    assert_eq!(body["per_page"], 2, "model default page size");
}

#[tokio::test]
async fn cursor_pagination_pages_from_records() {
    let (db, ctx, _) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    for i in 1..=7 {
        create_post(&db.db, ann.id, &format!("Post {i}")).await;
    }
    let titles = |body: &Value| -> Vec<String> {
        body["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["values"]["title"].as_str().unwrap().to_string())
            .collect()
    };
    let cursors = cursor_client(&ctx).await;
    let schema = cursors.get("/api/schema").await.json();
    assert_eq!(model(&schema, "posts")["pagination"], "cursor");
    let first = cursors
        .get("/api/models/posts/records?per_page=3")
        .await
        .json();
    assert_eq!(titles(&first), ["Post 7", "Post 6", "Post 5"]);
    assert_eq!(first["page"], Value::Null);
    assert_eq!(first["prev"], Value::Null);
    assert_eq!(first["next"], first["records"][2]["key"]);

    let next = first["next"].as_str().unwrap();
    let body = cursors
        .get(&format!(
            "/api/models/posts/records?per_page=3&after={next}"
        ))
        .await
        .json();
    assert_eq!(titles(&body), ["Post 4", "Post 3", "Post 2"]);
    assert_eq!(body["prev"], body["records"][0]["key"]);
    assert_eq!(body["next"], body["records"][2]["key"]);
    let prev = body["prev"].as_str().unwrap();
    let back = cursors
        .get(&format!(
            "/api/models/posts/records?per_page=3&before={prev}"
        ))
        .await
        .json();
    assert_eq!(back["records"], first["records"]);
    assert_eq!(back["prev"], Value::Null, "the first page has no previous");

    let next = body["next"].as_str().unwrap();
    let last = cursors
        .get(&format!(
            "/api/models/posts/records?per_page=3&after={next}"
        ))
        .await
        .json();
    assert_eq!(titles(&last), ["Post 1"]);
    assert_eq!(last["next"], Value::Null);
    let prev = last["prev"].as_str().unwrap();
    let back = cursors
        .get(&format!(
            "/api/models/posts/records?per_page=3&before={prev}"
        ))
        .await
        .json();
    assert_eq!(back["records"], body["records"]);

    assert_eq!(body["labels"]["user_id"][ann.id.to_string()], "ann");
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
    assert_eq!(body["records"], json!([]));
    let body = client.get("/api/models/users/count?q=hashed").await.json();
    assert_eq!(body["count"], exact(0));
    let body = client.get("/api/models/posts/count?q=rust").await.json();
    assert_eq!(body["count"], exact(1), "counts follow the search");
    let body = client
        .get("/api/models/posts/count?filter.rating=__null__")
        .await
        .json();
    assert_eq!(body["count"], exact(2), "and the filters");
    let res = client
        .get("/api/models/posts/count?filter.views=lots")
        .await;
    assert_eq!(res.status, 422);
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

    let too_many: Vec<String> = (0..=MAX_PER_PAGE).map(|i| i.to_string()).collect();
    let res = client
        .post("/api/models/posts/bulk-delete", json!({"keys": too_many}))
        .await;
    assert_eq!(res.status, 400);
    assert_eq!(
        res.json()["error"],
        "at most 500 records can be selected at once"
    );
    assert_eq!(count::<posts::Entity>(&db.db).await, 1, "nothing deleted");
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
    assert_eq!(res.status, 500);
    assert_eq!(res.json()["error"], "internal server error");
    assert!(!res.text().contains("hunter2"));

    let refuse = json!({"keys": [a.id.to_string()]});
    let res = client
        .post("/api/models/posts/actions/refuse", refuse.clone())
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json()["error"], "nothing to do");
    let res = client
        .post("/api/models/posts/actions/retitle", refuse)
        .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json()["error"], "rejected by hook");

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

    let too_many: Vec<String> = (0..=MAX_PER_PAGE).map(|i| i.to_string()).collect();
    let res = client
        .post(
            "/api/models/posts/actions/publish",
            json!({"keys": too_many}),
        )
        .await;
    assert_eq!(res.status, 400);
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
        json!({"users": exact(1), "posts": exact(1), "post_tags": exact(0), "notes": exact(0)})
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
            Admin::new(AllowAll).register(ModelAdmin::<posts::Entity>::new().list_filter(["meta"])),
            "JSON field `meta` cannot be used in list_filter",
        ),
        (
            Admin::new(AllowAll).register(ModelAdmin::<users::Entity>::new().show_fields(["x"])),
            "unknown field `x` in show_fields",
        ),
        (
            Admin::new(AllowAll).register(
                ModelAdmin::<users::Entity>::new()
                    .hidden_fields(["email"])
                    .show_fields(["email"]),
            ),
            "field `email` is in both hidden_fields and show_fields",
        ),
        (
            Admin::new(AllowAll).register(
                ModelAdmin::<users::Entity>::new()
                    .hidden_fields(["name"])
                    .list_display(["password"]),
            ),
            "hidden field `password` used in list_display",
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
        let err = admin.router(&ctx).await.expect_err("configuration error");
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
                .hidden_fields(["pid"])
                .show_fields(["api_key"])
                .readonly_fields(["email"])
                .list_display(["email", "api_key"])
                .search_fields(["api_key", "id"])
                .list_filter(["is_admin"])
                .display_field("email")
                .read_only()
                .can_export(true),
        )
        .router(&ctx)
        .await
        .unwrap();
    let client = Client::new(router);
    let ann = create_user(&db.db, "ann@example.com", true).await;

    let schema = client.get("/api/schema").await.json();
    let people = model(&schema, "people");
    assert_eq!(people["table"], "users");
    assert!(field(people, "api_key").is_some(), "shown on request");
    assert!(field(people, "pid").is_none(), "hidden on request");
    assert!(
        field(people, "password").is_none(),
        "still hidden by default"
    );
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
    assert_eq!(
        body["records"].as_array().unwrap().len(),
        1,
        "non-text search fields match their value"
    );
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

#[tokio::test]
async fn integers_beyond_javascript_precision_are_strings() {
    let (db, _ctx, client) = client().await;
    let user = create_user(&db.db, "ann@example.com", true).await;
    let mut input = new_post(user.id);
    input["views"] = json!("9007199254740993");
    let res = client.post("/api/models/posts/records", input).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.text());
    let record = res.json()["record"].clone();
    assert_eq!(record["values"]["views"], "9007199254740993");
    let key = record["key"].as_str().unwrap();
    let stored = posts::Entity::find_by_id(key.parse::<i32>().unwrap())
        .one(&db.db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.views, 9_007_199_254_740_993);

    // The UI sends the string back unchanged.
    let res = client
        .patch(
            &format!("/api/models/posts/records/{key}"),
            json!({"views": "9007199254740995"}),
        )
        .await;
    assert_eq!(res.json()["record"]["values"]["views"], "9007199254740995");
    let list = client
        .get("/api/models/posts/records?filter.views=9007199254740995")
        .await
        .json();
    assert_eq!(list["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        list["records"][0]["values"]["user_id"], user.id,
        "small stay numbers"
    );
}

#[tokio::test]
async fn search_ignores_case_of_non_ascii_letters() {
    let (db, _ctx, client) = client().await;
    let user = create_user(&db.db, "ann@example.com", true).await;
    for title in ["Москва", "Élan vital", "ÄRGER", "plain"] {
        create_post(&db.db, user.id, title).await;
    }
    for (q, expected) in [
        ("Москва", "Москва"),
        ("москва", "Москва"),
        ("МОСКВА", "Москва"),
        ("élan", "Élan vital"),
        ("Élan Vital", "Élan vital"),
        ("ärger", "ÄRGER"),
        ("ÄRGER", "ÄRGER"),
        ("PLAIN", "plain"),
    ] {
        let body = client
            .get(&format!(
                "/api/models/posts/records?q={}",
                percent_encoding::utf8_percent_encode(q, percent_encoding::NON_ALPHANUMERIC)
            ))
            .await
            .json();
        let titles: Vec<&str> = body["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["values"]["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, [expected], "q={q}");
    }
}

#[tokio::test]
async fn lookup_finds_records_by_key() {
    let (db, ctx) = setup().await;
    let router = Admin::new(AllowAll)
        .register(
            ModelAdmin::<posts::Entity>::new()
                .name("bare")
                .search_fields(Vec::<String>::new()),
        )
        .model::<posts::Entity>()
        .router(&ctx)
        .await
        .unwrap();
    let client = Client::new(router);
    let user = create_user(&db.db, "ann@example.com", true).await;
    let mut first = None;
    for i in 0..30 {
        let post = create_post(&db.db, user.id, &format!("post {i}")).await;
        first.get_or_insert(post.id);
    }
    let first = first.unwrap();
    for model in ["bare", "posts"] {
        let body = client
            .get(&format!("/api/models/{model}/lookup?q={first}"))
            .await
            .json();
        let keys: Vec<&str> = body["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["key"].as_str().unwrap())
            .collect();
        assert!(
            keys.contains(&first.to_string().as_str()),
            "{model}: {keys:?}"
        );
    }
    let body = client.get("/api/models/bare/lookup").await.json();
    assert_eq!(
        body["choices"].as_array().unwrap().len(),
        20,
        "newest first"
    );
}

#[tokio::test]
async fn out_of_range_pages_and_json_filters_are_bad_requests() {
    let (db, _ctx, client) = client().await;
    let user = create_user(&db.db, "ann@example.com", true).await;
    create_post(&db.db, user.id, "p").await;
    for uri in [
        "/api/models/posts/records?page=999999999999999999",
        "/api/models/posts/records?page=36893488147419103&per_page=500",
    ] {
        let res = client.get(uri).await;
        assert_eq!(res.status, 400, "{uri}: {}", res.text());
        assert_eq!(res.json()["error"], "`page` is too large");
    }
    let res = client
        .get("/api/models/posts/records?filter.meta=%7B%7D")
        .await;
    assert_eq!(res.status, 400, "{}", res.text());
    let res = client.get("/api/models/posts/records?sort=meta").await;
    assert_eq!(res.status, 200, "JSON sorts by its text: {}", res.text());
}

#[tokio::test]
async fn values_the_database_rejects_are_validation_errors() {
    use sea_orm::DbBackend;
    let (db, _ctx, client) = client().await;
    let user = create_user(&db.db, "ann@example.com", true).await;
    let postgres = db.db.get_database_backend() == DbBackend::Postgres;
    for (name, field, value) in [
        ("too large decimal", "price", json!("123456789012.50")),
        ("NUL in text", "title", json!("a\u{0}b")),
        ("NUL in JSON", "meta", json!({"k": "\u{0}"})),
    ] {
        let mut input = new_post(user.id);
        input[field] = value;
        let res = client.post("/api/models/posts/records", input).await;
        if postgres {
            assert_eq!(res.status, 422, "{name}: {}", res.text());
            assert_eq!(
                res.json()["error"],
                "the database rejected these values",
                "{name}"
            );
        } else {
            assert_eq!(res.status, StatusCode::CREATED, "{name}: {}", res.text());
        }
    }
}

/// Read `list` a page at a time, forward and then backward, and check both
/// walks see every record once, in the order of the list read in one page.
async fn assert_pages_walk(client: &Client, list: &str, per_page: u64) {
    let keys = |body: &Value| -> Vec<String> {
        body["records"]
            .as_array()
            .unwrap_or_else(|| panic!("{list}: {body}"))
            .iter()
            .map(|r| r["key"].as_str().unwrap().to_string())
            .collect()
    };
    let get = |cursor: String| {
        let uri = format!("{list}&per_page={per_page}{cursor}");
        let client = client.clone();
        async move { client.get(&uri).await.json() }
    };
    // Numbered pages go by `page`, cursor pagination by `after` and `before`.
    let numbered = client.get(&format!("{list}&per_page=1")).await.json()["page"].is_number();
    let cursor = |name: &str, key: &str| {
        let name = if numbered { "page" } else { name };
        format!("&{name}={}", utf8_percent_encode(key, NON_ALPHANUMERIC))
    };
    let all = keys(&client.get(&format!("{list}&per_page=500")).await.json());
    assert!(all.len() as u64 > per_page, "{list}: too few records");

    let mut page = get(String::new()).await;
    assert_eq!(page["prev"], Value::Null, "{list}: first page");
    let mut forward = Vec::new();
    while let Some(next) = page["next"].as_str() {
        assert_eq!(keys(&page).len() as u64, per_page, "{list}: a full page");
        forward.extend(keys(&page));
        page = get(cursor("after", next)).await;
        assert!(page["prev"].is_string(), "{list}: a later page");
    }
    forward.extend(keys(&page));
    assert_eq!(forward, all, "{list} forward by {per_page}");

    let mut backward = keys(&page);
    while let Some(prev) = page["prev"].as_str() {
        page = get(cursor("before", prev)).await;
        assert!(page["next"].is_string(), "{list}: an earlier page");
        backward.splice(0..0, keys(&page));
    }
    assert_eq!(backward, all, "{list} backward by {per_page}");
}

#[tokio::test]
async fn pages_walk_every_record_once_in_any_order() {
    use sea_orm::prelude::{ChronoDate, ChronoTime, Uuid};
    let (db, ctx, numbered) = client().await;
    let client = cursor_client(&ctx).await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let date = |d| ChronoDate::from_ymd_opt(2024, 1, d);
    // Ties and NULLs in every column, so the key has to break them.
    let rows = [
        ("b", None, 1, false, None, None),
        ("a", Some(2.0), 1, true, date(2), Some(json!({"k": 1}))),
        ("b", Some(2.0), 2, false, None, Some(json!({"k": 1}))),
        ("c", None, 2, true, date(1), None),
        ("a", Some(1.0), 3, false, date(1), Some(json!({"a": 2}))),
        ("b", None, 3, true, None, Some(json!({"k": 0}))),
        ("d", Some(3.0), 1, false, date(3), None),
        ("a", Some(2.0), 2, false, None, Some(json!({"k": 1}))),
        ("c", Some(1.0), 3, true, date(2), Some(json!({"z": [1]}))),
    ];
    for (i, (title, rating, views, published, day, meta)) in rows.into_iter().enumerate() {
        let mut am = create_post(&db.db, ann.id, title).await.into_active_model();
        am.rating = Set(rating);
        am.views = Set(views);
        am.featured = Set(i % 3 == 0);
        if published {
            am.status = Set(posts::PostStatus::Published);
        }
        am.published_on = Set(day);
        am.meta = Set(meta);
        am.update(&db.db).await.unwrap();
        for tag in ["x", "y"] {
            post_tags::ActiveModel {
                post_id: Set(i32::try_from(i + 1).unwrap()),
                tag: Set(tag.to_string()),
                weight: Set(i16::try_from(i % 2).unwrap()),
            }
            .insert(&db.db)
            .await
            .unwrap();
        }
        notes::ActiveModel {
            id: Set(Uuid::from_u128(u128::try_from(i).unwrap() * 7919 % 13)),
            content: Set(title.to_string()),
            reminder_at: Set(day.and_then(|d| d.and_hms_opt(9, 0, 0))),
            remind_time: Set(rating.map(|_| ChronoTime::from_hms_opt(8, 0, 0).unwrap())),
            pinned: Set(published.then_some(i % 2 == 0)),
            ..Default::default()
        }
        .insert(&db.db)
        .await
        .unwrap();
    }

    // Timestamps the database writes, as Loco's default for `created_at`
    // does, which SQLite keeps as text in another format than sqlx writes.
    insert_posts(&db.db, ann.id, 4).await;

    let mut lists = vec!["/api/models/posts/records?".to_string()];
    for field in [
        "id",
        "title",
        "rating",
        "views",
        "featured",
        "status",
        "published_on",
        "meta",
        "price",
        "created_at",
    ] {
        for order in ["asc", "desc"] {
            lists.push(format!(
                "/api/models/posts/records?sort={field}&order={order}"
            ));
        }
    }
    for order in ["asc", "desc"] {
        for field in ["weight", "tag", "post_id"] {
            lists.push(format!(
                "/api/models/post_tags/records?sort={field}&order={order}"
            ));
        }
        for field in ["content", "reminder_at", "remind_time", "pinned"] {
            lists.push(format!(
                "/api/models/notes/records?sort={field}&order={order}"
            ));
        }
    }
    lists.push("/api/models/posts/records?filter.featured=false&q=O&sort=rating".into());
    for list in &lists {
        for per_page in [2, 4] {
            assert_pages_walk(&client, list, per_page).await;
        }
    }
    for list in &lists[..3] {
        assert_pages_walk(&numbered, list, 4).await;
    }

    // NULLs sort where the database puts them, so its indexes serve the list.
    let body = client
        .get("/api/models/posts/records?sort=rating&order=asc&per_page=1")
        .await
        .json();
    let first = &body["records"][0]["values"]["rating"];
    if is_postgres() {
        assert_eq!(*first, json!(1.0), "Postgres sorts NULLs last");
    } else {
        assert_eq!(*first, Value::Null, "SQLite sorts NULLs first");
    }
}

#[tokio::test]
async fn cursors_whose_records_are_gone_still_find_a_page() {
    let (db, ctx, _) = client().await;
    let client = cursor_client(&ctx).await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let mut ids = Vec::new();
    for i in 1..=5 {
        ids.push(create_post(&db.db, ann.id, &format!("Post {i}")).await.id);
    }
    let page = |cursor: String| {
        let client = client.clone();
        async move {
            let body = client
                .get(&format!("/api/models/posts/records?per_page=2&{cursor}"))
                .await
                .json();
            let ids: Vec<i64> = body["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["values"]["id"].as_i64().unwrap())
                .collect();
            (ids, body["prev"].clone(), body["next"].clone())
        }
    };
    let id = |n: usize| i64::from(ids[n - 1]);

    let first = (vec![id(5), id(4)], Value::Null, json!(id(4).to_string()));
    assert_eq!(page("after=999999".into()).await, first, "no such record");
    assert_eq!(page("after=abc".into()).await, first, "not a key");
    assert_eq!(page(format!("before={}", id(5))).await, first);
    assert_eq!(
        page(format!("before={}", id(4))).await,
        first,
        "a short first page is filled"
    );

    // Deleting every record on the last page leaves the one before it.
    let res = client
        .delete(&format!("/api/models/posts/records/{}", id(1)))
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(
        page(format!("after={}", id(2))).await,
        (vec![id(3), id(2)], json!(id(3).to_string()), Value::Null)
    );
    // ... or the first page, when that is all there is.
    for n in [2, 3] {
        client
            .delete(&format!("/api/models/posts/records/{}", id(n)))
            .await;
    }
    assert_eq!(
        page(format!("after={}", id(4))).await,
        (vec![id(5), id(4)], Value::Null, Value::Null)
    );
}

#[tokio::test]
async fn counts_are_exact_unless_cursor_pagination_caps_them() {
    let (db, ctx, client) = client().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    if is_postgres() {
        // Postgres's estimate stays unknown until the test analyzes the table.
        db.db
            .execute_unprepared("ALTER TABLE posts SET (autovacuum_enabled = false)")
            .await
            .unwrap();
    }
    insert_posts(&db.db, ann.id, 10_050).await;
    assert_eq!(
        client.get("/api/models/posts/count").await.json()["count"],
        exact(10_050),
        "numbered pages count every record"
    );
    let stats = client.get("/api/stats").await.json();
    assert_eq!(
        stats["counts"]["posts"],
        exact(10_050),
        "so does the dashboard"
    );
    let cursors = cursor_client(&ctx).await;
    let count = |query: &'static str| {
        let client = cursors.clone();
        async move {
            client
                .get(&format!("/api/models/posts/count{query}"))
                .await
                .json()["count"]
                .clone()
        }
    };
    let more = json!({"kind": "more_than", "value": 10_000});
    assert_eq!(count("").await, more);
    assert_eq!(count("?q=bulk").await, more);
    assert_eq!(
        count("?filter.featured=true").await,
        exact(5_025),
        "counts below the limit are exact"
    );
    let stats = cursors.get("/api/stats").await.json();
    assert_eq!(stats["counts"]["posts"], more, "and its dashboard count");
    assert_eq!(stats["counts"]["users"], exact(1));

    if is_postgres() {
        db.db.execute_unprepared("ANALYZE posts").await.unwrap();
        let estimate = json!({"kind": "estimate", "value": 10_050});
        assert_eq!(count("").await, estimate);
        let stats = cursors.get("/api/stats").await.json();
        assert_eq!(stats["counts"]["posts"], estimate);
        assert_eq!(count("?q=bulk").await, more, "no estimate for a search");
    }
}

#[tokio::test]
async fn search_matches_other_fields_by_value() {
    let (db, ctx) = setup().await;
    let router = Admin::new(AllowAll)
        .register(ModelAdmin::<posts::Entity>::new().search_fields(["title", "views", "meta"]))
        .register(
            ModelAdmin::<posts::Entity>::new()
                .name("by_views")
                .search_fields(["views"]),
        )
        .register(ModelAdmin::<users::Entity>::new().search_fields(Vec::<String>::new()))
        .router(&ctx)
        .await
        .unwrap();
    let client = Client::new(router);
    let ann = create_user(&db.db, "ann@example.com", true).await;
    for (title, views, meta) in [
        ("Alpha", 5, None),
        ("Beta", 15, Some(json!({"tag": "rust"}))),
        ("Gamma 5", 50, None),
    ] {
        let mut am = create_post(&db.db, ann.id, title).await.into_active_model();
        am.views = Set(views);
        am.meta = Set(meta);
        am.update(&db.db).await.unwrap();
    }
    let titles = |model: &'static str, q: &'static str| {
        let client = client.clone();
        async move {
            let body = client
                .get(&format!("/api/models/{model}/records?sort=title&q={q}"))
                .await
                .json();
            body["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["values"]["title"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        titles("posts", "5").await,
        ["Alpha", "Gamma 5"],
        "numbers match whole values"
    );
    assert_eq!(titles("posts", "RUST").await, ["Beta"], "JSON by its text");
    assert_eq!(titles("by_views", "15").await, ["Beta"]);
    assert!(
        titles("by_views", "lots").await.is_empty(),
        "not a value of any search field"
    );
    let body = client.get("/api/models/by_views/count?q=lots").await.json();
    assert_eq!(body["count"], exact(0));

    // Pickers find records by a text value's start, or a number's value.
    let body = client
        .get("/api/models/users/lookup?column=email&q=ANN")
        .await
        .json();
    assert_eq!(body["choices"][0]["value"], "ann@example.com");
    let body = client.get("/api/models/users/lookup?q=x").await.json();
    assert_eq!(body["choices"], json!([]));
    let body = client
        .get(&format!("/api/models/users/lookup?q={}", ann.id))
        .await
        .json();
    assert_eq!(body["choices"][0]["value"], ann.id);
}

/// A fixed-length text column, which Postgres pads with spaces.
mod codes {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "codes")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(column_type = "Char(Some(5))")]
        pub code: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[tokio::test]
async fn pages_walk_padded_text() {
    let (db, ctx) = setup().await;
    let schema = sea_orm::Schema::new(db.db.get_database_backend());
    db.db
        .execute(&schema.create_table_from_entity(codes::Entity))
        .await
        .unwrap();
    db.db
        .execute_unprepared(
            "INSERT INTO codes (code) VALUES ('ab'), ('ab'), ('ab'), ('ab'), ('ab'), ('cd'), ('cd')",
        )
        .await
        .unwrap();
    let router = Admin::new(AllowAll)
        .register(ModelAdmin::<codes::Entity>::new().cursor_pagination())
        .router(&ctx)
        .await
        .unwrap();
    let client = Client::new(router);
    for order in ["asc", "desc"] {
        let list = format!("/api/models/codes/records?sort=code&order={order}");
        assert_pages_walk(&client, &list, 2).await;
    }
}

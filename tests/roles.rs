mod common;

use async_trait::async_trait;
use axum::{Router, http::request::Parts};
use common::{
    Client, JWT_SECRET, Migrator, TestDb, admin_roles, create_post, create_user, post_tags, posts,
    set_role, setup, users,
};
use loco_rs::{
    app::{AppContext, Initializer},
    auth::jwt::JWT,
};
use muy_loco_admin::{
    Admin, AdminAuth, AdminUser, AllowAll, AuthError, ModelAdmin, UserRoles, auth::JwtAuth,
};
use sea_orm::{ActiveModelTrait, ConnectionTrait, EntityTrait, Set, sea_query::Query};
use sea_orm_migration::MigratorTrait;
use serde_json::{Map, Value, json};

fn token(user: &users::Model) -> String {
    JWT::new(JWT_SECRET)
        .generate_token(3600, user.pid.to_string(), Map::new())
        .expect("token")
}

fn auth() -> JwtAuth<users::Model> {
    JwtAuth::<users::Model>::new(|user| user.is_admin)
        .display_name(|user| user.email.clone())
        .roles(users::Column::AdminRoleId, [users::Column::IsAdmin])
}

fn admin(auth: impl AdminAuth) -> Admin {
    Admin::new(auth)
        .register(ModelAdmin::<users::Entity>::new().can_create(false))
        .register(
            ModelAdmin::<posts::Entity>::new()
                .can_export(true)
                .action("publish", "Publish", |_, posts| async move {
                    Ok(format!("published {}", posts.len()))
                })
                .action("purge", "Purge", |_, _| async { Ok("purged".to_string()) }),
        )
        .model::<post_tags::Entity>()
}

struct World {
    db: TestDb,
    router: Router,
    full: Client,
    owner: users::Model,
}

async fn world() -> World {
    let (db, ctx) = setup().await;
    let router = admin(auth()).router(&ctx).expect("router");
    let owner = create_user(&db.db, "owner@example.com", true).await;
    let full = Client::new(router.clone()).with_token(token(&owner));
    World {
        db,
        router,
        full,
        owner,
    }
}

impl World {
    fn client(&self, user: &users::Model) -> Client {
        Client::new(self.router.clone()).with_token(token(user))
    }

    async fn role(&self, name: &str, permissions: Value) -> i64 {
        let res = self
            .full
            .post(
                "/api/roles",
                json!({"name": name, "permissions": permissions}),
            )
            .await;
        assert_eq!(res.status, 201, "{}", res.text());
        res.json()["role"]["id"].as_i64().unwrap()
    }

    /// An admin user limited to role `role`.
    async fn member(&self, email: &str, role: i64) -> users::Model {
        let user = create_user(&self.db.db, email, true).await;
        set_role(&self.db.db, &user, Some(role)).await;
        user
    }
}

fn model<'a>(schema: &'a Value, name: &str) -> Option<&'a Value> {
    schema["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
}

fn field<'a>(model: &'a Value, name: &str) -> &'a Value {
    model["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

#[tokio::test]
async fn full_admins_create_roles() {
    let w = world().await;
    let me = w.full.get("/api/me").await.json();
    assert_eq!(me["role"], Value::Null);
    assert_eq!(me["can_manage_roles"], true);
    assert_eq!(w.full.get("/api/roles").await.json(), json!({"roles": []}));

    // Permissions a model does not offer and models and actions that are not
    // registered are dropped; any permission implies view; models granted
    // nothing are left out.
    let res = w
        .full
        .post(
            "/api/roles",
            json!({
                "name": "  Editor ",
                "permissions": {
                    "posts": {"update": true, "actions": ["publish"]},
                    "users": {"create": true},
                    "post_tags": {"actions": ["publish"]},
                    "nope": {"view": true}
                }
            }),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.text());
    let editor = res.json()["role"].clone();
    assert_eq!(editor["name"], "Editor");
    assert_eq!(editor["users"], 0);
    assert_eq!(
        editor["permissions"],
        json!({"posts": {
            "view": true, "create": false, "update": true, "delete": false, "export": false,
            "actions": ["publish"]
        }})
    );

    for (body, field, message) in [
        (json!({"name": " "}), "name", "must not be empty"),
        (json!({}), "name", "must not be empty"),
        (
            json!({"name": "x".repeat(101)}),
            "name",
            "must be at most 100 characters",
        ),
        (json!({"name": "Editor"}), "name", "is already taken"),
    ] {
        let res = w.full.post("/api/roles", body.clone()).await;
        assert_eq!(res.status, 422, "{body}");
        assert_eq!(res.json()["fields"][field], json!([message]), "{body}");
    }
    let res = w
        .full
        .post(
            "/api/roles",
            json!({"name": "A", "permissions": {"posts": {"drop": true}}}),
        )
        .await;
    assert_eq!(res.status, 400);
    assert_eq!(
        w.full
            .post("/api/roles", json!({"name": "A", "color": "red"}))
            .await
            .status,
        400
    );
}

#[tokio::test]
async fn full_admins_update_and_delete_roles() {
    let w = world().await;
    let id = w
        .role(
            "Editor",
            json!({"posts": {"update": true, "actions": ["publish"]}}),
        )
        .await;
    let editor = w.full.get(&format!("/api/roles/{id}")).await.json()["role"].clone();
    let support = w
        .role("Support", json!({"users": {"view": true, "update": true}}))
        .await;
    let member = w.member("sam@example.com", support).await;
    let list = w.full.get("/api/roles").await.json();
    let names: Vec<(&str, u64)> = list["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["name"].as_str().unwrap(), r["users"].as_u64().unwrap()))
        .collect();
    assert_eq!(names, [("Editor", 0), ("Support", 1)]);

    let shown = w.full.get(&format!("/api/roles/{support}")).await.json();
    assert_eq!(shown["role"]["users"], 1);
    assert_eq!(w.full.get("/api/roles/999").await.status, 404);
    assert_eq!(w.full.get("/api/roles/abc").await.status, 404);

    // Renaming keeps the permissions; new permissions replace them.
    let res = w
        .full
        .patch(&format!("/api/roles/{id}"), json!({"name": "Writer"}))
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(res.json()["role"]["permissions"], editor["permissions"]);
    let res = w
        .full
        .patch(
            &format!("/api/roles/{id}"),
            json!({"permissions": {"post_tags": {"delete": true}}}),
        )
        .await;
    assert_eq!(res.json()["role"]["name"], "Writer");
    assert_eq!(
        res.json()["role"]["permissions"],
        json!({"post_tags": {
            "view": true, "create": false, "update": false, "delete": true, "export": false,
            "actions": []
        }})
    );
    let res = w
        .full
        .patch(&format!("/api/roles/{id}"), json!({"name": "Support"}))
        .await;
    assert_eq!(res.json()["fields"]["name"], json!(["is already taken"]));
    assert_eq!(
        w.full
            .patch("/api/roles/999", json!({"name": "Ghost"}))
            .await
            .status,
        404
    );

    // A role cannot be deleted while users have it.
    let res = w.full.delete(&format!("/api/roles/{support}")).await;
    assert_eq!(res.status, 409);
    assert_eq!(
        res.json()["error"],
        "this role is assigned to 1 user; give them another role first"
    );
    // Nor directly in the database: the foreign key restricts it.
    let direct = admin_roles::Entity::delete_by_id(support)
        .exec(&w.db.db)
        .await;
    assert!(direct.is_err(), "the role column restricts deletes");

    set_role(&w.db.db, &member, None).await;
    let res = w.full.delete(&format!("/api/roles/{support}")).await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(
        w.full.delete(&format!("/api/roles/{support}")).await.status,
        404
    );
}

#[tokio::test]
async fn full_admins_assign_roles_on_the_users_table() {
    let w = world().await;
    let editor = w.role("Editor", json!({"posts": {"view": true}})).await;
    let user = create_user(&w.db.db, "ed@example.com", true).await;

    let schema = w.full.get("/api/schema").await.json();
    let users = model(&schema, "users").unwrap();
    let role = field(users, "admin_role_id");
    assert_eq!(role["role"], true);
    assert_eq!(role["readonly"], false);
    assert!(role.get("relation").is_none(), "picked from the roles");
    assert!(field(users, "email").get("role").is_none());

    let res = w
        .full
        .patch(
            &format!("/api/models/users/records/{}", user.id),
            json!({"admin_role_id": editor.to_string()}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(
        res.json()["labels"]["admin_role_id"],
        json!({ editor.to_string(): "Editor" })
    );
    let list = w.full.get("/api/models/users/records").await.json();
    assert_eq!(
        list["labels"]["admin_role_id"],
        json!({ editor.to_string(): "Editor" })
    );

    // Assigning a role that does not exist breaks the foreign key.
    let res = w
        .full
        .patch(
            &format!("/api/models/users/records/{}", user.id),
            json!({"admin_role_id": "999"}),
        )
        .await;
    assert_eq!(res.status, 409, "{}", res.text());

    let me = w.client(&user).get("/api/me").await.json();
    assert_eq!(
        me,
        json!({
            "id": user.pid.to_string(),
            "name": "ed@example.com",
            "role": {"id": editor, "name": "Editor"},
            "can_manage_roles": false
        })
    );
}

/// A world with a post and an editor, who may view and update posts, publish
/// them and view post tags.
async fn editor_world() -> (World, i64, Client, posts::Model) {
    let w = world().await;
    let editor = w
        .role(
            "Editor",
            json!({
                "posts": {"view": true, "update": true, "actions": ["publish"]},
                "post_tags": {"view": true}
            }),
        )
        .await;
    let ed = w.member("ed@example.com", editor).await;
    let client = w.client(&ed);
    let post = create_post(&w.db.db, w.owner.id, "Hello").await;
    (w, editor, client, post)
}

#[tokio::test]
async fn a_role_limits_what_a_user_sees() {
    let (w, _, client, _) = editor_world().await;

    let schema = client.get("/api/schema").await.json();
    let names: Vec<&str> = schema["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["posts", "post_tags"]);
    let posts = model(&schema, "posts").unwrap();
    assert_eq!(
        posts["capabilities"],
        json!({"view": true, "create": false, "update": true, "delete": false, "export": false})
    );
    assert_eq!(
        posts["actions"],
        json!([{"name": "publish", "label": "Publish"}])
    );
    // Users are hidden from this role, so their key is a plain value.
    assert_eq!(field(posts, "user_id")["relation"]["model"], Value::Null);
    assert_eq!(
        posts["related"],
        json!([{"model": "post_tags", "field": "post_id", "label": "Post tags"}])
    );
    assert_eq!(
        model(&schema, "post_tags").unwrap()["capabilities"]["update"],
        false
    );
    let stats = client.get("/api/stats").await.json();
    assert_eq!(stats["counts"], json!({"posts": 1, "post_tags": 0}));

    // Labels of users are not shown.
    let list = client.get("/api/models/posts/records").await;
    assert_eq!(list.status, 200);
    assert!(list.json()["labels"].get("user_id").is_none());
    let full_list = w.full.get("/api/models/posts/records").await.json();
    assert_eq!(
        full_list["labels"]["user_id"][w.owner.id.to_string()],
        "owner"
    );
}

#[tokio::test]
async fn a_role_limits_what_a_user_does() {
    let (w, editor, client, post) = editor_world().await;
    let path = format!("/api/models/posts/records/{}", post.id);
    let res = client.patch(&path, json!({"title": "Edited"})).await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert!(res.json()["labels"].get("user_id").is_none());
    let res = client
        .post(
            "/api/models/posts/actions/publish",
            json!({"keys": [post.id.to_string()]}),
        )
        .await;
    assert_eq!(res.json()["message"], "published 1");

    for (method, uri, body) in [
        (
            "POST",
            "/api/models/posts/records",
            Some(json!({"title": "New"})),
        ),
        ("DELETE", path.as_str(), None),
        (
            "POST",
            "/api/models/posts/bulk-delete",
            Some(json!({"keys": [post.id.to_string()]})),
        ),
        ("GET", "/api/models/posts/export", None),
        (
            "POST",
            "/api/models/posts/actions/purge",
            Some(json!({"keys": [post.id.to_string()]})),
        ),
        ("GET", "/api/models/users/records", None),
        ("GET", "/api/models/users/lookup?q=o", None),
        (
            "GET",
            &format!("/api/models/users/records/{}", w.owner.id),
            None,
        ),
        (
            "PATCH",
            "/api/models/post_tags/records/1,a",
            Some(json!({"weight": 1})),
        ),
        ("GET", "/api/roles", None),
        ("POST", "/api/roles", Some(json!({"name": "Mine"}))),
        ("GET", &format!("/api/roles/{editor}"), None),
        (
            "PATCH",
            &format!("/api/roles/{editor}"),
            Some(json!({"name": "Boss"})),
        ),
        ("DELETE", &format!("/api/roles/{editor}"), None),
    ] {
        let res = client.send(method.parse().unwrap(), uri, body).await;
        assert_eq!(res.status, 403, "{method} {uri}: {}", res.text());
    }
    // Unknown actions stay unknown.
    let res = client
        .post("/api/models/posts/actions/missing", json!({"keys": ["1"]}))
        .await;
    assert_eq!(res.status, 404);
}

#[tokio::test]
async fn only_full_admins_change_who_may_use_the_admin() {
    let w = world().await;
    let support = w.role("Support", json!({"users": {"update": true}})).await;
    let sam = w.member("sam@example.com", support).await;
    let client = w.client(&sam);
    let regular = create_user(&w.db.db, "joe@example.com", false).await;

    let schema = client.get("/api/schema").await.json();
    let users = model(&schema, "users").unwrap();
    assert_eq!(field(users, "is_admin")["readonly"], true);
    assert_eq!(field(users, "admin_role_id")["readonly"], true);
    assert_eq!(field(users, "email")["readonly"], false);
    assert_eq!(users["capabilities"]["update"], true);

    for (user, change) in [
        (&sam, json!({"admin_role_id": null})),
        (&regular, json!({"is_admin": true})),
        (
            &regular,
            json!({"name": "Joe", "admin_role_id": support.to_string()}),
        ),
    ] {
        let res = client
            .patch(
                &format!("/api/models/users/records/{}", user.id),
                change.clone(),
            )
            .await;
        assert_eq!(res.status, 422, "{change}");
        assert_eq!(res.json()["error"], "some fields are invalid");
    }
    let res = client
        .patch(
            &format!("/api/models/users/records/{}", regular.id),
            json!({"name": "Joe"}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    // Role names are labels, not secrets.
    let res = client
        .get(&format!("/api/models/users/records/{}", sam.id))
        .await;
    assert_eq!(
        res.json()["labels"]["admin_role_id"],
        json!({ support.to_string(): "Support" })
    );
}

#[tokio::test]
async fn a_role_with_nothing_shows_nothing() {
    let w = world().await;
    let empty = w.role("Empty", json!({})).await;
    let user = w.member("nobody@example.com", empty).await;
    let client = w.client(&user);
    assert_eq!(
        client.get("/api/schema").await.json(),
        json!({"models": []})
    );
    assert_eq!(client.get("/api/stats").await.json(), json!({"counts": {}}));
}

fn user_roles() -> UserRoles {
    UserRoles {
        table: "users".to_string(),
        column: "admin_role_id".to_string(),
        protected: vec!["is_admin".to_string()],
    }
}

/// Signs everyone in with the role it was given, with roles on or off.
struct Fixed(Option<i64>, Option<UserRoles>);

#[async_trait]
impl AdminAuth for Fixed {
    async fn authenticate(&self, _: &AppContext, _: &Parts) -> Result<AdminUser, AuthError> {
        Ok(AdminUser::new("u", "U").with_role(self.0))
    }

    fn roles(&self) -> Option<UserRoles> {
        self.1.clone()
    }
}

/// Insert a role as something other than the admin might have saved it.
async fn raw_role(db: &sea_orm::DatabaseConnection, name: &str, permissions: &str) -> i64 {
    let now = sea_orm::prelude::ChronoUtc::now().fixed_offset();
    admin_roles::ActiveModel {
        name: Set(name.to_string()),
        permissions: Set(permissions.to_string()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap()
    .id
}

#[tokio::test]
async fn missing_and_unreadable_roles_grant_nothing() {
    let (db, ctx) = setup().await;
    let missing = Client::new(
        admin(Fixed(Some(999), Some(user_roles())))
            .router(&ctx)
            .unwrap(),
    );
    assert_eq!(missing.get("/api/me").await.status, 403);
    assert_eq!(missing.get("/api/schema").await.status, 403);

    let broken = raw_role(&db.db, "Broken", "not json").await;
    let client = Client::new(
        admin(Fixed(Some(broken), Some(user_roles())))
            .router(&ctx)
            .unwrap(),
    );
    assert_eq!(
        client.get("/api/schema").await.json(),
        json!({"models": []})
    );
    assert_eq!(client.get("/api/roles").await.status, 403);

    // Full admins can still list, fix and delete it.
    let full = Client::new(admin(Fixed(None, Some(user_roles()))).router(&ctx).unwrap());
    let res = full.get("/api/roles").await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(
        res.json()["roles"],
        json!([{"id": broken, "name": "Broken", "permissions": {}, "users": 0}])
    );
    let res = full.get(&format!("/api/roles/{broken}")).await;
    assert_eq!(res.json()["role"]["permissions"], json!({}));
    let res = full
        .patch(
            &format!("/api/roles/{broken}"),
            json!({"permissions": {"posts": {"view": true}}}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(
        client.get("/api/schema").await.json()["models"][0]["name"],
        "posts"
    );
    let res = full.delete(&format!("/api/roles/{broken}")).await;
    assert_eq!(res.status, 200, "{}", res.text());
}

#[tokio::test]
async fn roles_naming_removed_models_can_be_saved() {
    let w = world().await;
    let stale = raw_role(
        &w.db.db,
        "Stale",
        r#"{"gone": {"view": true}, "posts": {"view": true, "actions": ["publish", "retired"]}}"#,
    )
    .await;
    let role = w.full.get(&format!("/api/roles/{stale}")).await.json()["role"].clone();
    // The UI sends back what it was given, with the new name.
    let res = w
        .full
        .patch(
            &format!("/api/roles/{stale}"),
            json!({"name": "Fresh", "permissions": role["permissions"]}),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.text());
    assert_eq!(res.json()["role"]["name"], "Fresh");
    assert_eq!(
        res.json()["role"]["permissions"],
        json!({"posts": {
            "view": true, "create": false, "update": false, "delete": false, "export": false,
            "actions": ["publish"]
        }})
    );
}

#[tokio::test]
async fn a_role_needs_the_auth_to_keep_roles() {
    let (db, ctx) = setup().await;
    let editor = raw_role(
        &db.db,
        "Editor",
        r#"{"users": {"view": true, "update": true}}"#,
    )
    .await;
    let user = create_user(&db.db, "editor@example.com", true).await;
    set_role(&db.db, &user, Some(editor)).await;
    // The auth hands out a role but never says where roles are kept, so the
    // admin cannot protect the role column: it refuses rather than let the
    // user change their own role.
    let client = Client::new(admin(Fixed(Some(editor), None)).router(&ctx).unwrap());
    assert_eq!(client.get("/api/me").await.status, 500);
    let res = client
        .patch(
            &format!("/api/models/users/records/{}", user.id),
            json!({"admin_role_id": null}),
        )
        .await;
    assert_eq!(res.status, 500);
    // Users without a role are unaffected.
    let full = Client::new(admin(Fixed(None, None)).router(&ctx).unwrap());
    assert_eq!(full.get("/api/me").await.status, 200);
}

#[tokio::test]
async fn roles_are_off_unless_the_auth_keeps_them() {
    let (_db, ctx) = setup().await;
    let client = Client::new(admin(AllowAll).router(&ctx).unwrap());
    assert_eq!(client.get("/api/roles").await.status, 404);
    assert_eq!(
        client.post("/api/roles", json!({"name": "x"})).await.status,
        404
    );
    let schema = client.get("/api/schema").await.json();
    let users = model(&schema, "users").unwrap();
    assert!(field(users, "admin_role_id").get("role").is_none());
    let me = client.get("/api/me").await.json();
    assert_eq!(me["can_manage_roles"], false);
}

/// Roles kept by a custom auth, which declares where.
fn custom() -> Fixed {
    Fixed(None, Some(user_roles()))
}

#[tokio::test]
async fn mounting_checks_the_roles_tables() {
    let (db, ctx) = setup().await;
    let router = Router::new();
    assert!(
        admin(custom())
            .after_routes(router.clone(), &ctx)
            .await
            .is_ok()
    );

    Migrator::down(&db.db, None).await.unwrap();
    let err = admin(custom())
        .after_routes(router.clone(), &ctx)
        .await
        .expect_err("missing tables");
    assert!(
        err.to_string()
            .contains("add `muy_loco_admin::migrations::roles(\"users\")` to the app's `Migrator`"),
        "{err}"
    );
    // Mounting by hand cannot check, so the app asks.
    assert!(admin(custom()).router(&ctx).is_ok());
    let err = admin(custom())
        .check_database(&ctx)
        .await
        .expect_err("missing tables");
    assert!(
        err.to_string()
            .contains("roles need the `admin_roles` table"),
        "{err}"
    );
    assert!(admin(AllowAll).check_database(&ctx).await.is_ok());
    // Without roles, nothing is checked.
    assert!(
        admin(AllowAll)
            .after_routes(router.clone(), &ctx)
            .await
            .is_ok()
    );
    // The migration can be run again after rolling it back.
    Migrator::up(&db.db, None).await.unwrap();
    assert!(exists(&db.db, "admin_roles", "id").await);
    assert!(exists(&db.db, "users", "admin_role_id").await);
}

#[tokio::test]
async fn migrations_roll_back() {
    let (db, _ctx) = setup().await;
    Migrator::down(&db.db, None).await.unwrap();
    assert!(!exists(&db.db, "admin_roles", "id").await);
    assert!(!exists(&db.db, "users", "admin_role_id").await);
    // Users are left as the app's own migrations made them.
    assert_eq!(insert_user_row(&db.db).await, 1);
}

/// Whether `table` has `column`.
async fn exists(
    db: &sea_orm::DatabaseConnection,
    table: &'static str,
    column: &'static str,
) -> bool {
    // Qualified, since SQLite reads an unknown quoted name as a string.
    let query = Query::select()
        .column((table, column))
        .from(table)
        .limit(1)
        .to_owned();
    db.query_one(&query).await.is_ok()
}

async fn insert_user_row(db: &sea_orm::DatabaseConnection) -> u64 {
    let insert = Query::insert()
        .into_table("users")
        .columns([
            "created_at",
            "updated_at",
            "pid",
            "email",
            "name",
            "password",
            "api_key",
            "is_admin",
        ])
        .values_panic([
            sea_orm::prelude::ChronoUtc::now().fixed_offset().into(),
            sea_orm::prelude::ChronoUtc::now().fixed_offset().into(),
            sea_orm::prelude::Uuid::nil().into(),
            "a@example.com".into(),
            "a".into(),
            "x".into(),
            "k".into(),
            true.into(),
        ])
        .to_owned();
    db.execute(&insert).await.unwrap().rows_affected()
}

#[tokio::test]
async fn misconfigured_roles_fail_at_mount() {
    let (_db, ctx) = setup().await;
    // The role column must be the migration's nullable bigint column.
    let wrong = JwtAuth::<users::Model>::new(|user| user.is_admin)
        .roles(users::Column::Id, [users::Column::IsAdmin]);
    let err = admin(wrong).router(&ctx).expect_err("wrong column");
    assert!(err.to_string().contains("the role column `id`"), "{err}");

    // The roles table is managed on the Roles page, never as a model.
    let err = admin(auth())
        .model::<admin_roles::Entity>()
        .router(&ctx)
        .expect_err("roles registered");
    assert!(
        err.to_string()
            .contains("the `admin_roles` table is managed on the admin's Roles page"),
        "{err}"
    );
    // Without roles, an app's own `admin_roles` table is just a table.
    assert!(
        admin(AllowAll)
            .model::<admin_roles::Entity>()
            .router(&ctx)
            .is_ok()
    );
}

//! Test entities, database setup and an HTTP client for the admin router.
//!
//! Tests run against SQLite in memory unless `DATABASE_URL` points at
//! Postgres, in which case each test gets its own fresh database.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use loco_rs::{app::AppContext, config::Config, environment::Environment};
use sea_orm::{
    ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend,
    EntityTrait, Schema, Set,
};
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const JWT_SECRET: &str = "c2VjcmV0LWZvci10ZXN0aW5nLW11eS1sb2NvLWFkbWluLTEyMzQ1Njc4OTA=";

pub mod users {
    use loco_rs::model::{Authenticable, ModelError, ModelResult};
    use sea_orm::{QueryFilter, entity::prelude::*};

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        // Timestamps first, as in the entities Loco generates.
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
        #[sea_orm(primary_key)]
        pub id: i32,
        pub pid: Uuid,
        #[sea_orm(unique)]
        pub email: String,
        pub name: String,
        pub password: String,
        pub api_key: String,
        pub is_admin: bool,
        /// Added by `muy_loco_admin::migrations::roles`.
        pub admin_role_id: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "super::posts::Entity")]
        Posts,
        // As `sea-orm-cli` generates it once the roles table exists.
        #[sea_orm(
            belongs_to = "super::admin_roles::Entity",
            from = "Column::AdminRoleId",
            to = "super::admin_roles::Column::Id"
        )]
        AdminRoles,
    }

    impl Related<super::posts::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Posts.def()
        }
    }

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
        where
            C: ConnectionTrait,
        {
            Ok(super::stamp(self, insert, |m| {
                (&mut m.created_at, &mut m.updated_at)
            }))
        }
    }

    #[async_trait::async_trait]
    impl Authenticable for Model {
        async fn find_by_api_key(db: &DatabaseConnection, api_key: &str) -> ModelResult<Self> {
            Entity::find()
                .filter(Column::ApiKey.eq(api_key))
                .one(db)
                .await?
                .ok_or(ModelError::EntityNotFound)
        }

        async fn find_by_claims_key(
            db: &DatabaseConnection,
            claims_key: &str,
        ) -> ModelResult<Self> {
            if claims_key == "explode" {
                return Err(ModelError::Message("lookup failed".into()));
            }
            let pid = Uuid::parse_str(claims_key).map_err(|_| ModelError::EntityNotFound)?;
            Entity::find()
                .filter(Column::Pid.eq(pid))
                .one(db)
                .await?
                .ok_or(ModelError::EntityNotFound)
        }
    }
}

/// `users` as the app's own migrations create it, before the admin's.
mod app_users {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
        #[sea_orm(primary_key)]
        pub id: i32,
        pub pid: Uuid,
        #[sea_orm(unique)]
        pub email: String,
        pub name: String,
        pub password: String,
        pub api_key: String,
        pub is_admin: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// The roles table as `sea-orm-cli` generates it in an app.
pub mod admin_roles {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "admin_roles")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique)]
        pub name: String,
        #[sea_orm(column_type = "Text")]
        pub permissions: String,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod posts {
    use sea_orm::entity::prelude::*;

    #[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
    #[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "post_status")]
    pub enum PostStatus {
        #[sea_orm(string_value = "draft")]
        Draft,
        #[sea_orm(string_value = "published")]
        Published,
    }

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "posts")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub title: String,
        #[sea_orm(column_type = "Text")]
        pub body: String,
        pub status: PostStatus,
        pub featured: bool,
        pub rating: Option<f64>,
        #[sea_orm(column_type = "Decimal(Some((10, 2)))")]
        pub price: Decimal,
        pub views: i64,
        pub published_on: Option<Date>,
        #[sea_orm(column_type = "JsonBinary", nullable)]
        pub meta: Option<Json>,
        pub user_id: i32,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::users::Entity",
            from = "Column::UserId",
            to = "super::users::Column::Id"
        )]
        User,
        #[sea_orm(has_many = "super::post_tags::Entity")]
        PostTags,
    }

    impl Related<super::users::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::User.def()
        }
    }

    impl Related<super::post_tags::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::PostTags.def()
        }
    }

    #[async_trait::async_trait]
    impl ActiveModelBehavior for ActiveModel {
        async fn before_save<C>(self, _db: &C, insert: bool) -> Result<Self, DbErr>
        where
            C: ConnectionTrait,
        {
            if let sea_orm::ActiveValue::Set(title) = &self.title {
                if title.trim().is_empty() {
                    return Err(DbErr::Custom(
                        r#"{"title":[{"code":"length","message":"title must not be empty"}]}"#
                            .to_string(),
                    ));
                }
                if title == "hook-error" {
                    return Err(DbErr::Custom("rejected by hook".to_string()));
                }
            }
            Ok(super::stamp(self, insert, |m| {
                (&mut m.created_at, &mut m.updated_at)
            }))
        }

        async fn before_delete<C>(self, _db: &C) -> Result<Self, DbErr>
        where
            C: ConnectionTrait,
        {
            if self.title.as_ref() == "locked" {
                return Err(DbErr::Custom("locked posts cannot be deleted".to_string()));
            }
            Ok(self)
        }
    }
}

pub mod post_tags {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "post_tags")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub post_id: i32,
        #[sea_orm(primary_key, auto_increment = false)]
        pub tag: String,
        pub weight: i16,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::posts::Entity",
            from = "Column::PostId",
            to = "super::posts::Column::Id"
        )]
        Post,
    }

    impl Related<super::posts::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Post.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod notes {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub content: String,
        pub reminder_at: Option<DateTime>,
        pub remind_time: Option<Time>,
        pub pinned: Option<bool>,
        pub secret_token: Option<String>,
        #[sea_orm(column_type = "Blob", nullable)]
        pub attachment: Option<Vec<u8>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Set `created_at`/`updated_at` like Loco's generated models do.
fn stamp<A>(
    mut am: A,
    insert: bool,
    fields: impl Fn(
        &mut A,
    ) -> (
        &mut sea_orm::ActiveValue<sea_orm::prelude::DateTimeWithTimeZone>,
        &mut sea_orm::ActiveValue<sea_orm::prelude::DateTimeWithTimeZone>,
    ),
) -> A {
    let now = sea_orm::prelude::ChronoUtc::now().fixed_offset();
    let (created, updated) = fields(&mut am);
    if insert {
        if created.is_not_set() {
            *created = Set(now);
        }
        if updated.is_not_set() {
            *updated = Set(now);
        }
    } else if updated.is_unchanged() {
        *updated = Set(now);
    }
    am
}

/// An app's `Migrator`, with only the admin's migrations.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![muy_loco_admin::migrations::roles("users")]
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| "sqlite::memory:".to_string())
}

pub fn is_postgres() -> bool {
    database_url().starts_with("postgres")
}

async fn connect() -> (DatabaseConnection, String) {
    let url = database_url();
    if url.starts_with("postgres") {
        let admin = Database::connect(&url).await.expect("connect to postgres");
        let name = format!(
            "mla_{}_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
            sea_orm::prelude::ChronoUtc::now().timestamp_subsec_nanos()
        );
        admin
            .execute_unprepared(&format!("CREATE DATABASE \"{name}\""))
            .await
            .expect("create test database");
        admin.close().await.ok();
        let (base, query) = url
            .split_once('?')
            .map_or((url.as_str(), None), |(b, q)| (b, Some(q)));
        let root = base.rsplit_once('/').map_or(base, |(root, _)| root);
        let test_url = match query {
            Some(q) => format!("{root}/{name}?{q}"),
            None => format!("{root}/{name}"),
        };
        let db = Database::connect(&test_url)
            .await
            .expect("connect to test database");
        (db, test_url)
    } else {
        let mut options = ConnectOptions::new(url.clone());
        options
            .max_connections(1)
            .min_connections(1)
            .sqlx_logging(false);
        let db = Database::connect(options).await.expect("connect to sqlite");
        (db, url)
    }
}

async fn create_schema(db: &DatabaseConnection) {
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);
    if backend == DbBackend::Postgres {
        let stmt = schema
            .create_enum_from_active_enum::<posts::PostStatus>()
            .expect("enum statement");
        db.execute(&stmt).await.expect("create enum");
    }
    // The app's own migrations create `users` without a role column; the
    // admin's migration adds it.
    db.execute(&schema.create_table_from_entity(app_users::Entity))
        .await
        .expect("create users");
    Migrator::up(db, None).await.expect("admin migrations");
    db.execute(&schema.create_table_from_entity(posts::Entity))
        .await
        .expect("create posts");
    db.execute(&schema.create_table_from_entity(post_tags::Entity))
        .await
        .expect("create post_tags");
    db.execute(&schema.create_table_from_entity(notes::Entity))
        .await
        .expect("create notes");
}

pub fn config(uri: &str, extra: &Value) -> Config {
    let mut base = json!({
        "logger": { "enable": false, "level": "error", "format": "compact" },
        "server": { "port": 5150, "host": "http://localhost" },
        "database": {
            "uri": uri,
            "enable_logging": false,
            "min_connections": 1,
            "max_connections": 1,
            "connect_timeout": 500,
            "idle_timeout": 500,
            "auto_migrate": false,
            "dangerously_truncate": false,
            "dangerously_recreate": false
        },
        "auth": { "jwt": { "secret": JWT_SECRET, "expiration": 3600 } }
    });
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in extra {
            base.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(base).expect("valid config")
}

pub struct TestDb {
    pub db: DatabaseConnection,
    pub uri: String,
}

pub async fn database() -> TestDb {
    let (db, uri) = connect().await;
    create_schema(&db).await;
    TestDb { db, uri }
}

pub fn context(test_db: &TestDb, env: Environment, extra: &Value) -> AppContext {
    AppContext::builder(env, test_db.db.clone(), config(&test_db.uri, extra)).build()
}

pub async fn setup() -> (TestDb, AppContext) {
    let test_db = database().await;
    let ctx = context(&test_db, Environment::Test, &json!({}));
    (test_db, ctx)
}

pub async fn create_user(db: &DatabaseConnection, email: &str, is_admin: bool) -> users::Model {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    users::ActiveModel {
        pid: Set(sea_orm::prelude::Uuid::from_u128(u128::from(n) + 1_000)),
        email: Set(email.to_string()),
        name: Set(email.split('@').next().unwrap_or(email).to_string()),
        password: Set("hashed".to_string()),
        api_key: Set(format!("key-{n}")),
        is_admin: Set(is_admin),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("insert user")
}

/// Give `user` role `role`, or full access with `None`.
pub async fn set_role(db: &DatabaseConnection, user: &users::Model, role: Option<i64>) {
    users::ActiveModel {
        id: Set(user.id),
        admin_role_id: Set(role),
        ..Default::default()
    }
    .update(db)
    .await
    .expect("set role");
}

pub async fn create_post(db: &DatabaseConnection, user_id: i32, title: &str) -> posts::Model {
    posts::ActiveModel {
        title: Set(title.to_string()),
        body: Set(format!("Body of {title}")),
        status: Set(posts::PostStatus::Draft),
        featured: Set(false),
        rating: Set(None),
        price: Set(sea_orm::prelude::Decimal::new(1999, 2)),
        views: Set(0),
        published_on: Set(None),
        meta: Set(None),
        user_id: Set(user_id),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("insert post")
}

pub async fn count<E: EntityTrait>(db: &DatabaseConnection) -> u64
where
    E::Model: Sync,
{
    use sea_orm::PaginatorTrait;
    E::find().count(db).await.expect("count")
}

/// A response from the admin router.
pub struct Res {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Res {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("invalid JSON ({e}): {}", self.text()))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// Sends requests to a router, adding the CSRF header and an optional token.
#[derive(Clone)]
pub struct Client {
    pub router: Router,
    pub token: Option<String>,
    pub csrf: bool,
}

impl Client {
    pub fn new(router: Router) -> Self {
        Self {
            router,
            token: None,
            csrf: true,
        }
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    pub fn without_csrf(mut self) -> Self {
        self.csrf = false;
        self
    }

    pub async fn send(&self, method: Method, uri: &str, body: Option<Value>) -> Res {
        self.send_raw(method, uri, body.map(|b| b.to_string()), vec![])
            .await
    }

    pub async fn send_raw(
        &self,
        method: Method,
        uri: &str,
        body: Option<String>,
        headers: Vec<(&str, &str)>,
    ) -> Res {
        let mut req = Request::builder().method(method).uri(uri);
        if self.csrf {
            req = req.header(muy_loco_admin::CSRF_HEADER, muy_loco_admin::CSRF_VALUE);
        }
        if let Some(token) = &self.token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let req = match body {
            Some(body) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body)),
            None => req.body(Body::empty()),
        }
        .expect("request");
        let res = self.router.clone().oneshot(req).await.expect("response");
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.into_body().collect().await.expect("body").to_bytes();
        Res {
            status,
            headers,
            body,
        }
    }

    pub async fn get(&self, uri: &str) -> Res {
        self.send(Method::GET, uri, None).await
    }

    pub async fn post(&self, uri: &str, body: Value) -> Res {
        self.send(Method::POST, uri, Some(body)).await
    }

    pub async fn patch(&self, uri: &str, body: Value) -> Res {
        self.send(Method::PATCH, uri, Some(body)).await
    }

    pub async fn delete(&self, uri: &str) -> Res {
        self.send(Method::DELETE, uri, None).await
    }
}

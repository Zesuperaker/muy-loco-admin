//! An admin dashboard for [Loco](https://loco.rs) apps, generated at runtime from your `SeaORM`
//! entities.
//!
//! ![The articles list in the admin](https://raw.githubusercontent.com/zesuperaker/muy-loco-admin/main/docs/screenshots/list.png)
//!
//! **New here?** The [Quickstart](https://github.com/zesuperaker/muy-loco-admin#quickstart)
//! adds the admin to an app in a few steps. This page is the guide to everything after that.
//!
//! ```rust,ignore
//! use muy_loco_admin::{auth::JwtAuth, Admin, ModelAdmin};
//!
//! // src/app.rs, inside `impl Hooks for App`
//! async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
//!     Ok(vec![Box::new(
//!         Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true)))
//!             .register(ModelAdmin::<users::Entity>::new().can_create(false))
//!             .model::<articles::Entity>()
//!             .middlewares_from::<Self>(),
//!     )])
//! }
//! ```
//!
//! # Features
//!
//! - Lists with pagination, sorting, search and filters
//! - Create, edit and delete, plus bulk delete and [custom actions](#custom-actions)
//! - Writes go through `ActiveModel`, so model hooks and validation run as they do in app code
//! - Foreign keys as links and searchable pickers, with links to related records
//! - CSV export, streamed from one consistent snapshot
//! - Composite and UUID primary keys, Postgres enums, JSON columns
//! - Optional [roles](#roles), managed in the UI, that limit what each admin user can do
//! - Postgres and SQLite
//! - The React UI is embedded in the crate, so there is nothing to build or serve separately
//!
//! # Registering models
//!
//! Every model you want in the admin is registered on [`Admin`], either with the defaults or
//! with a [`ModelAdmin`] that configures it:
//!
//! ```rust,ignore
//! use muy_loco_admin::{Admin, ModelAdmin, SortDirection};
//!
//! Admin::new(auth)
//!     // The defaults: every column, all operations.
//!     .model::<categories::Entity>()
//!     // Configured.
//!     .register(
//!         ModelAdmin::<articles::Entity>::new()
//!             .list_display(["id", "title", "status", "user_id", "published_at"])
//!             .search_fields(["title", "body"])
//!             .list_filter(["status", "user_id"])
//!             .ordering("published_at", SortDirection::Desc)
//!             .can_delete(false),
//!     )
//! ```
//!
//! Fields are named by column name, as strings (`articles::Column::Title.as_str()` works too).
//! A field the entity does not have stops the admin from starting, with the model and option
//! named in the error.
//!
//! A foreign key becomes a link and a searchable picker when the model it points to is
//! registered too, and that model's records link back to the records that refer to them.
//! Otherwise it is a plain id field.
//!
//! ## Generating the model list
//!
//! The [`AdminModels`] task writes the list for you. Register it with the app's tasks:
//!
//! ```rust,ignore
//! // src/app.rs, inside `impl Hooks for App`
//! fn register_tasks(tasks: &mut Tasks) {
//!     tasks.register(AdminModels::new());
//! }
//! ```
//!
//! Then run `cargo loco task admin:models`. The first run creates `src/admin.rs`, with a
//! `models` function that registers each entity in `src/models/_entities`, and declares the
//! module in `src/lib.rs`. Pass the app's [`Admin`] through it:
//!
//! ```rust,ignore
//! let admin = Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true)));
//! Box::new(crate::admin::models(admin).middlewares_from::<Self>())
//! ```
//!
//! The file is yours to edit:
//!
//! - Comment a line out to leave that model out of the admin.
//! - Change `.model::<…>()` to `.register(ModelAdmin::<…>::new()…)` to configure the model.
//!
//! After adding tables and running `cargo loco db entities`, run the task again. It adds a line
//! for each entity the file does not mention yet, above the
//! `// admin-models-inject (do not remove this comment)` line, and leaves the rest alone. So a
//! new table never appears in the admin until you run the task, and the lines it adds show up in
//! your diff.
//!
//! The task finds an entity in the file by its `name::Entity` path, commented out or not. It
//! skips the admin's own `admin_roles` table. A model with a required hidden column, such as
//! `users.password`, is added with `can_create(false)`, since its records could not be created
//! from the admin. [`AdminModels::entities`] and [`AdminModels::output`] change where the task
//! reads and writes.
//!
//! # Model options
//!
//! | [`ModelAdmin`] method | Default |
//! | --- | --- |
//! | [`name`](ModelAdmin::name) | table name; used in URLs |
//! | [`label`](ModelAdmin::label) | humanized table name |
//! | [`list_display`](ModelAdmin::list_display) | all columns except text, JSON and binary; key first, timestamps last |
//! | [`search_fields`](ModelAdmin::search_fields) | text columns |
//! | [`list_filter`](ModelAdmin::list_filter) | boolean, enum and foreign-key columns |
//! | [`readonly_fields`](ModelAdmin::readonly_fields) | `created_at`, `updated_at`; auto-increment keys are always read-only |
//! | [`hidden_fields`](ModelAdmin::hidden_fields) | none; columns containing `password`, `secret`, `token`, `api_key` or `salt` are always hidden too |
//! | [`show_fields`](ModelAdmin::show_fields) | none; shows such a column anyway (e.g. `prompt_tokens`) |
//! | [`ordering`](ModelAdmin::ordering) | primary key, descending |
//! | [`per_page`](ModelAdmin::per_page) | 25, at most [`MAX_PER_PAGE`] (500) |
//! | [`display_field`](ModelAdmin::display_field) | first of `name`, `title`, `label`, `email`, `username`, `slug`, `code`; labels the record in links and pickers |
//! | [`can_create`](ModelAdmin::can_create), [`can_update`](ModelAdmin::can_update), [`can_delete`](ModelAdmin::can_delete), [`can_export`](ModelAdmin::can_export) | `true` |
//! | [`read_only`](ModelAdmin::read_only) | off; turns off create, update and delete |
//! | [`action`](ModelAdmin::action) | none; see [Custom actions](#custom-actions) |
//!
//! Hidden fields are never sent to or accepted from the UI. A model's primary key cannot be
//! hidden.
//!
//! # Custom actions
//!
//! An action runs on the records selected in the list and returns the message shown to the
//! user:
//!
//! ```rust,ignore
//! use loco_rs::app::AppContext;
//! use sea_orm::{ActiveModelTrait, ActiveValue, IntoActiveModel};
//!
//! use crate::models::articles::ArticleStatus;
//!
//! ModelAdmin::<articles::Entity>::new().action(
//!     "publish",  // name, unique per model
//!     "Publish",  // button label
//!     |ctx: AppContext, articles: Vec<articles::Model>| async move {
//!         let count = articles.len();
//!         for article in articles {
//!             let mut article = article.into_active_model();
//!             article.status = ActiveValue::Set(ArticleStatus::Published);
//!             article.published_at = ActiveValue::Set(Some(chrono::Utc::now().into()));
//!             article.update(&ctx.db).await?;
//!         }
//!         Ok(format!("Published {count} articles"))
//!     },
//! )
//! ```
//!
//! To fail with a message of your own, return `loco_rs::Error::BadRequest(message)`. Database
//! errors are reported as for the admin's own writes; any other error is logged and shown as an
//! internal error, without its details. With [roles](#roles) on, each action is a permission of
//! its own.
//!
//! # Configuration
//!
//! The admin can be configured in code or in the Loco config. The config wins, so each
//! environment can differ:
//!
//! ```yaml
//! initializers:
//!   admin:
//!     enabled: true     # false leaves the admin unmounted
//!     path: /admin
//!     title: Admin
//!     site_url: /       # the "Visit site" link: a path or http(s) URL; "" hides it
//! ```
//!
//! | In code | Default |
//! | --- | --- |
//! | [`Admin::path`] | `/admin`; letters, digits, `-`, `_`, `.` and `~` between single slashes |
//! | [`Admin::title`] | `Admin` |
//! | [`Admin::site_url`] | `/` |
//! | [`Admin::export_idle_timeout`] | 60 s; a CSV download that stalls this long is stopped and its transaction rolled back |
//!
//! To mount the admin yourself instead of as an initializer, see [`Admin::router`]; the
//! `initializers.admin` config does not apply then.
//!
//! # Authentication
//!
//! Every admin request is authenticated by the [`AdminAuth`] passed to [`Admin::new`]. The
//! crate ships two:
//!
//! - [`JwtAuth`](auth::JwtAuth) uses Loco's JWT auth (`auth.jwt` in the config). Its check, such
//!   as `user.is_admin`, decides who may use the admin and runs on every request.
//! - [`AllowAll`] lets anyone in, for local development. It runs only in a debug build in the
//!   `development` or `test` environment and logs a warning when mounted; anywhere else,
//!   including any release build, the admin refuses to start.
//!
//! Admin users without a [role](#roles) may do everything each model's [`ModelAdmin`] allows.
//!
//! ## The login form
//!
//! By default the UI shows a login form that posts `email` and `password` as JSON to
//! `/api/auth/login` and reads `token` from the reply, like Loco's `SaaS` starter. It then
//! trades the token for a [session](#sessions) cookie and never stores the token itself.
//!
//! ```rust,ignore
//! use muy_loco_admin::{auth::JwtAuth, FormLogin, LoginMethod};
//!
//! // A different login route:
//! JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true))
//!     .login_endpoint("/api/v2/login")
//!     .display_name(|user| user.email.clone()); // shown in the UI instead of the pid
//!
//! // Different field names:
//! JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true)).login_method(
//!     LoginMethod::Form(FormLogin {
//!         identifier_field: "username".into(),
//!         identifier_label: "Username".into(),
//!         ..FormLogin::default()
//!     }),
//! );
//!
//! // Your app's own login page, which must sign the user in with a Loco session
//! // (or set a token cookie listed in `auth.jwt.location`):
//! JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true))
//!     .login_method(LoginMethod::Redirect { url: "/login".into() });
//! ```
//!
//! Scripts can skip the form and send a token as `Authorization: Bearer`, or in the locations
//! set in `auth.jwt.location`.
//!
//! ## Your own authentication
//!
//! For anything else, implement [`AdminAuth`]. Return an [`AdminUser`] only for users who may
//! use the admin:
//!
//! ```rust,ignore
//! use axum::http::request::Parts;
//! use loco_rs::{app::AppContext, prelude::async_trait};
//! use muy_loco_admin::{AdminAuth, AdminUser, AuthError, LoginMethod};
//!
//! struct MyAuth;
//!
//! #[async_trait]
//! impl AdminAuth for MyAuth {
//!     async fn authenticate(
//!         &self,
//!         ctx: &AppContext,
//!         parts: &Parts,
//!     ) -> Result<AdminUser, AuthError> {
//!         let user = current_user(ctx, parts).await.ok_or(AuthError::Unauthenticated)?;
//!         if !user.is_staff {
//!             return Err(AuthError::Forbidden);
//!         }
//!         Ok(AdminUser::new(user.pid.to_string(), user.name))
//!     }
//!
//!     fn login(&self) -> LoginMethod {
//!         LoginMethod::Redirect { url: "/login".into() }
//!     }
//! }
//! ```
//!
//! [`AuthError::Unauthenticated`] shows the login screen and [`AuthError::Forbidden`] tells the
//! user their account has no admin access.
//!
//! The admin checks the session settings only when it shows its login form. If your
//! `AdminAuth` reads Loco's session, set a session secret of your own (see
//! [Deploying to production](#deploying-to-production)): without one, Loco signs sessions with a
//! public default outside the `production` environment, and anyone could forge one.
//!
//! ## Requests from other sites
//!
//! API requests that pages on other sites make are refused (`Sec-Fetch-Site`), and requests
//! that change data must send [`CSRF_HEADER`]`: `[`CSRF_VALUE`]
//! (`X-Requested-With: muy-loco-admin`). The UI does both; scripts that write must send the
//! header.
//!
//! # Sessions
//!
//! The login form signs users in with Loco's session (`auth.session`): an `HttpOnly`,
//! `SameSite=Lax` cookie, `Secure` when `server.host` is `https://`. The app shares it, so
//! signing in or out of the admin does the same in the app. Two things follow:
//!
//! - The admin reads the session's user id as the user's `pid`, so every
//!   `Session::set_user_id` in your app, OAuth's `on_sign_in` included, must pass the `pid`.
//!   Any other id, such as a username, could sign someone in as the user whose `pid` it happens
//!   to match.
//! - Every way your app signs users in (password, OAuth, magic links, remember-me) also signs
//!   admin users into the admin. The [`JwtAuth`](auth::JwtAuth) check still runs on every
//!   request.
//!
//! With the default `cookie` store, a copied cookie keeps working until `max_age`, even after
//! sign-out or a password change. A server-side store such as `database` ends the session on
//! sign-out; then also end a user's sessions wherever your app changes a password:
//!
//! ```rust,ignore
//! loco_rs::auth::session_store::sign_out_everywhere(&ctx, &user.pid.to_string()).await?;
//! ```
//!
//! See [Deploying to production](#deploying-to-production) for the settings to use.
//!
//! # Roles
//!
//! Roles limit what an admin user can do. They are created on the admin's Roles page and given
//! to a user on the user's record.
//!
//! - A role grants, per model, any of view, create, edit, delete and export, plus each custom
//!   action. It never grants more than the model's [`ModelAdmin`] allows.
//! - Admin users without a role have full access. Only they can manage roles and assign them.
//! - Models a role cannot view are hidden, and foreign keys to them become plain id fields. The
//!   role editor warns about this.
//! - A role cannot be deleted while users have it.
//!
//! To turn roles on:
//!
//! 1. Add the crate to the migration crate (`cd migration && cargo add muy-loco-admin`), and
//!    its roles migration to the end of the list in `migration/src/lib.rs`:
//!
//!    ```rust,ignore
//!    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
//!        vec![
//!            // ...the app's migrations
//!            muy_loco_admin::migrations::roles("users"),
//!            // inject-above (do not remove this comment)
//!        ]
//!    }
//!    ```
//!
//!    It creates an `admin_roles` table and gives `users` a nullable `admin_role_id` column.
//!    Migrations generated later go after it, so `cargo loco db down` rolls back the newest
//!    first.
//!
//! 2. Migrate and regenerate the entities:
//!
//!    ```sh
//!    cargo loco db migrate
//!    cargo loco db entities
//!    ```
//!
//! 3. Tell [`JwtAuth`](auth::JwtAuth::roles) the role column and the columns its check reads:
//!
//!    ```rust,ignore
//!    JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true))
//!        .roles(users::Column::AdminRoleId, [users::Column::IsAdmin])
//!    ```
//!
//! Only admin users with full access may change those columns, so a user with a role cannot
//! hand out access. A role that can edit users can still change their other columns, such as a
//! full admin's email, so grant it only to people you would trust with those accounts; the role
//! editor warns about this too.
//!
//! Do not register the generated `admin_roles` entity as a model; the admin refuses to start if
//! you do. With your own [`AdminAuth`], return [`UserRoles`] from [`AdminAuth::roles`] (requests
//! from users with a role fail without it) and the user's role with [`AdminUser::with_role`].
//!
//! # Middleware
//!
//! Admin requests go through the app's Loco middleware stack, with three exceptions:
//!
//! - `fallback` and `static` are skipped, since they add routes that would shadow the admin's.
//! - `cors` is skipped: the UI is always served from the same origin as its API, so no other
//!   origin may call it with the session cookie.
//! - The admin sets its own `Content-Security-Policy`, `X-Frame-Options` and `Referrer-Policy`,
//!   and drops `Clear-Site-Data`.
//!
//! [`Admin::middlewares_from::<Self>()`](Admin::middlewares_from) uses the app's
//! `Hooks::middlewares()`; without it, the stack configured under `server.middlewares` is used.
//!
//! # Deploying to production
//!
//! - **Authentication**: use [`JwtAuth`](auth::JwtAuth) or your own [`AdminAuth`].
//!   [`AllowAll`] refuses to start in a release build or outside `development` and `test`.
//! - **Session secret**: set `auth.session.secret` to a long random value
//!   (`openssl rand -base64 64`). With the login form, the admin accepts Loco's public
//!   development default only in a debug build in the `development` or `test` environment, as
//!   for [`AllowAll`].
//! - **Cookie**: serve the app over `https://` and name the cookie `__Host-…`, which stops
//!   subdomains planting a session.
//! - **Server-side sessions**: use `store: database` so sign-out ends the session, keep Loco's
//!   `auth_session` middleware in the stack, and call `sign_out_everywhere` wherever a password
//!   changes (see [Sessions](#sessions)).
//! - **User ids**: every `Session::set_user_id` in your app passes the user's `pid`.
//!
//! ```yaml
//! auth:
//!   session:
//!     secret: <%= get_env(name="SESSION_SECRET") %>
//!     cookie_name: __Host-loco_session
//!     store: database
//! ```
//!
//! # Troubleshooting
//!
//! The admin checks its setup when the app starts and refuses to mount if something would not
//! work or would be unsafe. The error says what to change. The first three rows are checked
//! only when the admin shows its login form.
//!
//! | The error mentions | Fix |
//! | --- | --- |
//! | `` set `auth.session.secret` `` | Set a session secret of your own (see [above](#deploying-to-production)). |
//! | `` session cookie unless it is `Secure` `` | A `__Host-` or `__Secure-` cookie needs an `https://` `server.host`, or `auth.session.secure: true`. |
//! | `` no `auth_session` middleware `` | A server-side session store needs Loco's `auth_session` middleware: keep it in `Hooks::middlewares` and use [`middlewares_from::<Self>()`](Admin::middlewares_from). |
//! | `AllowAll lets anyone use the admin` | Use [`JwtAuth`](auth::JwtAuth) or your own [`AdminAuth`] outside local development. |
//! | `unknown field` / `hidden field` | A [`ModelAdmin`] option names a column the entity lacks, or one that is hidden. |
//! | `registered more than once` | Give one of the models a different [`name`](ModelAdmin::name). |
//! | `` roles need the `admin_roles` table `` | Add [`migrations::roles`] to the app's `Migrator` and run `cargo loco db migrate`. |
//! | `must be the nullable bigint column` | Run `cargo loco db entities` after migrating. |
//! | `cannot be registered` | Remove the `admin_roles` model; roles are managed on the Roles page. |
//! | `` run `cargo loco db entities` first `` | `admin:models` found no entities: generate them, or point [`AdminModels::entities`] at them. |
//! | `` has no `// admin-models-inject `` | `admin:models` is writing to a file without the marker comment: add it after the models, or use [`AdminModels::output`]. |
//!
//! If you can sign in but see "Your account does not have admin access", the user fails the
//! [`JwtAuth`](auth::JwtAuth) check: run `cargo loco task admin:promote email:...` (see
//! [`PromoteAdmin`]).

// The guide links to `JwtAuth`, which only exists with the default `jwt` feature.
#![cfg_attr(not(feature = "jwt"), allow(rustdoc::broken_intra_doc_links))]

mod admin;
mod api;
mod assets;
pub mod auth;
mod error;
mod generate;
mod meta;
mod model;
mod query;
mod resource;
mod roles;
mod session;
mod task;
mod value;

pub use admin::{Admin, CONFIG_KEY};
pub use api::{CSRF_HEADER, CSRF_VALUE};
pub use auth::{AdminAuth, AdminUser, AllowAll, AuthError, FormLogin, LoginMethod, UserRoles};
pub use error::{AdminError, FieldErrors};
pub use generate::{AdminModels, MODELS_MARKER};
pub use meta::{FieldKind, SortDirection};
pub use model::{ActionResult, ModelAdmin};
pub use query::{MAX_PER_PAGE, NOT_NULL_FILTER, NULL_FILTER};
pub use task::PromoteAdmin;

/// The admin's migrations, to add to the app's `Migrator`.
///
/// Add each at the end of the list, above `inject-above`, so the list stays
/// in the order the migrations ran and `cargo loco db down` rolls back the
/// newest first. Later versions may add migrations here; add them the same
/// way.
pub mod migrations {
    pub use crate::roles::migration as roles;
}

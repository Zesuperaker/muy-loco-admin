# muy-loco-admin

An admin dashboard for [Loco](https://loco.rs) apps, generated at runtime from your SeaORM
entities.

## Quickstart

Requires Loco 1.3+, Rust 1.94+ and Postgres or SQLite. The steps use an app from Loco's SaaS
starter.

> [!TIP]
> Starting a new app? Create it from the SaaS starter:
>
> ```sh
> cargo install loco
> loco new --name myapp --db sqlite --bg async --assets serverside
> ```
>
> Use `--db postgres` for Postgres. Every Loco template with a database includes the user auth
> these steps rely on.

Add the crate:

<!-- x-release-please-start-version -->
```toml
[dependencies]
muy-loco-admin = "0.1.0"
```
<!-- x-release-please-end -->

Give `users` an `is_admin` column (`db entities` runs `sea-orm-cli`):

```sh
cargo install sea-orm-cli
cargo loco generate migration AddIsAdminToUsers is_admin:bool
cargo loco db migrate
cargo loco db entities
```

Add the admin to the app's initializers and its `admin:promote` task to the app's tasks, in
`src/app.rs`:

```rust
use muy_loco_admin::{auth::JwtAuth, Admin, ModelAdmin, PromoteAdmin};

// inside `impl Hooks for App`
async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
    Ok(vec![
        // ...the app's existing initializers
        Box::new(
            Admin::new(
                JwtAuth::<users::Model>::new(|user| user.is_admin == Some(true))
                    .login_endpoint("/api/auth/login"), // your app's login route
            )
            .register(ModelAdmin::<users::Entity>::new().can_create(false))
            // .model::<articles::Entity>() for each of your other models
            .middlewares_from::<Self>(),
        ),
    ])
}

fn register_tasks(tasks: &mut Tasks) {
    // ...the app's existing tasks
    tasks.register(PromoteAdmin::new(users::Column::Email, users::Column::IsAdmin));
}
```

Create a user (skip if you have one), make them an admin and start the app:

```sh
cargo loco task user:create email:you@example.com name:You password:change-me
cargo loco task admin:promote email:you@example.com
cargo loco start
```

Open `/admin` and sign in. The login form posts `email` and `password` as JSON to the login
endpoint and reads `token` from the reply, like the Loco SaaS starter's `/api/auth/login` (the
default), then trades the token for a session cookie. For other field names, or to send users to
your own login page, use `JwtAuth::login_method`. Before deploying, configure the
[session](#sessions).

Not on the SaaS starter? Create users however your app does, then run `admin:promote`. If your
app does not use Loco's JWT auth, implement `AdminAuth` (see [Authentication](#authentication)).

## Features

- List with pagination, sorting, search and filters; click a row to open its record
- Create, edit and delete; bulk delete and custom bulk actions
- Writes go through `ActiveModel`, so model hooks and validation run
- Foreign keys as links and searchable pickers; links to related records
- CSV export, streamed in primary-key order from one consistent snapshot; a download that
  stalls for `Admin::export_idle_timeout` (60 s) is stopped and its transaction rolled back
- Composite and UUID primary keys, Postgres enums, JSON columns
- The React UI is embedded in the crate; nothing to build or serve separately

## Model options

| `ModelAdmin` method | Default |
| --- | --- |
| `name` | table name |
| `label` | humanized table name |
| `list_display` | all columns except text, JSON and binary; key first, timestamps last |
| `search_fields` | text columns |
| `list_filter` | boolean, enum and foreign-key columns |
| `readonly_fields` | `created_at`, `updated_at` (auto-increment keys are always read-only) |
| `hidden_fields` | none; columns containing `password`, `secret`, `token`, `api_key` or `salt` are always hidden too |
| `show_fields` | none; shows such a column anyway (e.g. `prompt_tokens`) |
| `ordering` | primary key, descending |
| `per_page` | 25 (max 500) |
| `display_field` | first of `name`, `title`, `label`, `email`, `username`, `slug`, `code`; linked in the list |
| `can_create`, `can_update`, `can_delete`, `can_export`, `read_only` | everything allowed |
| `action(name, label, handler)` | none |

## Authentication

- `JwtAuth<U: Authenticable>` uses Loco's `auth.jwt` config. The UI trades the token from the
  login form for a [session](#sessions) cookie and never stores it. Scripts can send a token
  instead, as `Authorization: Bearer` or from `auth.jwt.location`.
- `AllowAll` allows anonymous access in the `development` and `test` environments only; every
  other environment (`production`, `staging`, ...) is refused.
- For anything else, implement `AdminAuth` and return `AdminUser::new(id, name)`.

Every admin user may do everything; what can be done to each model is set with `ModelAdmin`.

API requests made by pages on other sites are refused (`Sec-Fetch-Site`), and mutating requests
must send `X-Requested-With: muy-loco-admin`; the UI does both.

### Sessions

The login form signs users in with Loco's session (`auth.session`): an `HttpOnly`,
`SameSite=Lax` cookie, `Secure` when `server.host` is `https://`. The app shares it, so signing in
or out of the admin does the same in the app; the user id is the user's `pid`. For production:

```yaml
auth:
  session:
    secret: <%= get_env(name="SESSION_SECRET") %> # required outside development and test
    cookie_name: __Host-loco_session # needs https; stops subdomains planting a session
    store: database # sign-out ends the session on the server
```

With the default `cookie` store a copied cookie keeps working until `max_age`, even after
sign-out. The admin refuses to mount when the login form could not work: no secret outside
development and test, a `__Host-` or `__Secure-` cookie without `Secure`, or a server-side store
without Loco's `auth_session` middleware.

## Middleware

Admin requests go through the app's Loco middleware stack, except `fallback` and `static`, and
`cors`: the UI is always served from the same origin as its API, so no other origin may call it
with the session cookie.
`middlewares_from::<Self>()` uses the app's `Hooks::middlewares()`; without it, the stack
configured under `server.middlewares` is used. The admin sets its own `Content-Security-Policy`,
`X-Frame-Options` and `Referrer-Policy`, and drops `Clear-Site-Data`.

## Configuration

Optional, in the Loco config:

```yaml
initializers:
  admin:
    enabled: true
    path: /admin
    title: Admin
    site_url: /          # "Visit site" link: a path or http(s) URL; "" hides it
```

`Admin::path`, `Admin::title` and `Admin::site_url` set the same in code.

## Development

| Path | Contents |
| --- | --- |
| `src/` | the crate |
| `frontend/` | the UI (React, Vite); `frontend/dist` is embedded at build time and committed |
| `dogfood/` | a Loco app using the crate, run with Docker |
| `dogfood/e2e/` | Playwright tests against the dogfood app |

```sh
cargo test                                         # SQLite
DATABASE_URL=postgres://user:pass@localhost:5432/postgres cargo test   # Postgres

cd frontend && npm ci && npm run lint && npm run typecheck && npm test
npm run build                                      # then commit frontend/dist

cd dogfood && docker compose up --build            # http://localhost:5150/admin
cd dogfood/e2e && npm ci && npx playwright install chromium && npm test
```

Commit messages and PR titles follow [Conventional Commits](https://www.conventionalcommits.org),
and branches are named `<type>/<topic>` (`feat/site-link`). After CI passes on `main`, release-please keeps a release PR up to date with the next version and
changelog; merging it tags the release and publishes the crate to crates.io. Below 1.0, `fix:`
and `feat:` bump the patch version and breaking changes (`feat!:`) the minor.

Dogfood logins (password `12341234`): `user1@example.com` (admin), `user2@example.com`
(no admin access). The database is reseeded on every start; set `SEED_ON_START=false` to keep
data. Run the dogfood Rust tests with `cargo test` in `dogfood/` against a Postgres at
`DATABASE_URL` (default `postgres://loco:loco@localhost:5432/dogfood_test`).

## License

Apache-2.0

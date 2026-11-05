# muy-loco-admin

An auto-generated admin dashboard for [Loco](https://loco.rs) apps. It reads SeaORM entity
metadata at runtime and serves an embedded React UI. Writes go through `ActiveModel`, so model
hooks and validation run.

- List with pagination, sorting, search and filters
- Create, edit and delete; bulk delete and custom bulk actions
- Foreign keys as links and searchable pickers; links to related records
- CSV export, streamed in primary-key order from one consistent snapshot
- Composite and UUID primary keys, Postgres enums, JSON columns

Requires Loco 1.2+, Rust 1.94+ and Postgres or SQLite.

## Usage

```toml
[dependencies]
muy-loco-admin = "0.1"
```

```rust
// src/app.rs, inside `impl Hooks for App`
use muy_loco_admin::{auth::JwtAuth, Admin, ModelAdmin};

async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
    Ok(vec![Box::new(
        Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin))
            .register(ModelAdmin::<users::Entity>::new().can_create(false))
            .model::<articles::Entity>()
            .middlewares_from::<Self>(),
    )])
}
```

Open `/admin` and sign in. The login form posts to `/api/auth/login` (the Loco SaaS starter
endpoint); change it with `JwtAuth::login_method`.

### Model options

| `ModelAdmin` method | Default |
| --- | --- |
| `name` | table name |
| `label` | humanized table name |
| `list_display` | all columns except text, JSON and binary |
| `search_fields` | text columns |
| `list_filter` | boolean, enum and foreign-key columns |
| `readonly_fields` | `created_at`, `updated_at` (auto-increment keys are always read-only) |
| `hidden_fields` | columns containing `password`, `secret`, `token`, `api_key` or `salt` |
| `ordering` | primary key, descending |
| `per_page` | 25 (max 500) |
| `display_field` | first of `name`, `title`, `label`, `email`, `username`, `slug`, `code` |
| `can_create`, `can_update`, `can_delete`, `can_export`, `read_only` | everything allowed |
| `action(name, label, handler)` | none |

### Authentication

- `JwtAuth<U: Authenticable>` uses Loco's `auth.jwt` config. The token is read from the
  configured locations or `Authorization: Bearer`. Restrict operations per model with
  `.permissions(|user, access| ...)`.
- `AllowAll` allows anonymous access for development and is refused in `production`.
- For anything else, implement `AdminAuth` and return
  `AdminUser::new(id, name).with_policy(...)`.

Mutating API requests must send `X-Requested-With: muy-loco-admin`; the UI does this.

### Middleware

Admin requests go through the app's Loco middleware stack, except `fallback` and `static`.
`middlewares_from::<Self>()` uses the app's `Hooks::middlewares()`; without it, the stack
configured under `server.middlewares` is used. The admin sets its own `Content-Security-Policy`,
`X-Frame-Options` and `Referrer-Policy`, and drops `Clear-Site-Data`.

### Configuration

Optional, in the Loco config:

```yaml
initializers:
  admin:
    enabled: true
    path: /admin
    title: Admin
```

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

Dogfood logins (password `12341234`): `user1@example.com` (admin), `user2@example.com`
(no admin access). The database is reseeded on every start; set `SEED_ON_START=false` to keep
data. Run the dogfood Rust tests with `cargo test` in `dogfood/` against a Postgres at
`DATABASE_URL` (default `postgres://loco:loco@localhost:5432/dogfood_test`).

## License

Apache-2.0

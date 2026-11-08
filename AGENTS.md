# AGENTS.md

Admin dashboard crate for Loco apps. See `README.md` for usage.

## Layout

- `src/` crate: `admin.rs` (builder, initializer), `api.rs` (HTTP handlers), `resource.rs`
  (SeaORM operations), `value.rs` (value/JSON conversion), `session.rs` (Loco session sign-in),
  `roles.rs` (roles, their migration and permission checks), `auth.rs`, `meta.rs`, `assets.rs`
- `tests/` crate integration tests; `tests/common` holds test entities and the HTTP client
- `frontend/` React UI; `frontend/dist` is embedded with `rust-embed` and committed
- `dogfood/` Loco app using the crate (Postgres, Docker); `dogfood/e2e/` Playwright tests

## Checks

Run before committing; CI runs the same.

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test                                   # SQLite; set DATABASE_URL=postgres://... for Postgres
cd frontend && npm run lint && npm run typecheck && npm run coverage && npm run build
cd dogfood && cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test
cd dogfood && docker compose up --build -d && cd e2e && npm test
```

## Rules

- Support only the databases Loco supports: Postgres and SQLite. Crate tests must pass on both.
- After any UI change, run `npm run build` and commit `frontend/dist`. Only `index.html` may
  contain the `/__MLA_BASE__` placeholder.
- Crate dependencies use loco-rs's version requirement as the floor; crates loco-rs does not use
  take their latest release as the floor. Dev dependencies, the dogfood app, npm packages,
  Docker images and GitHub Actions are pinned to exact latest versions; `release.yml` pins
  actions by commit SHA, with the version in a comment.
- `rust-version` stays at loco-rs's (1.94).
- Do not add low-star or unmaintained dependencies.
- Keep coverage at or above the CI thresholds (Rust 90% lines, frontend 85% lines).
- Commit messages and PR titles follow Conventional Commits (`feat: ...`, `fix(auth): ...`);
  release-please derives versions and `CHANGELOG.md` from them. Never edit the crate version by
  hand.
- Name branches `<type>/<topic>`, with a Conventional Commits type and a short kebab-case topic
  (`feat/site-link`, `fix/export-timeout`), not generated or session names.
- Keep `README.md` and this file short.

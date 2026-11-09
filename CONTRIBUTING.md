# Contributing

## Layout

| Path | Contents |
| --- | --- |
| `src/` | the crate; the guide published on docs.rs is the crate docs in `src/lib.rs` |
| `tests/` | crate integration tests; `tests/common` holds test entities and the HTTP client |
| `frontend/` | the UI (React, Vite); `frontend/dist` is embedded at build time and committed |
| `dogfood/` | a Loco app using the crate, run with Docker |
| `dogfood/e2e/` | Playwright tests against the dogfood app |
| `docs/screenshots/` | the screenshots in the README and on docs.rs, taken from the dogfood app |

## Checks

CI runs these; run them from the repository root before opening a pull request.

```sh
# Crate
cargo fmt --check && cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo test                                                            # SQLite
DATABASE_URL=postgres://user:pass@localhost:5432/postgres cargo test  # Postgres

# Frontend: after any UI change, commit the rebuilt frontend/dist
(cd frontend && npm ci && npm run lint && npm run typecheck && npm run coverage && npm run build)

# Dogfood app: Rust tests against a Postgres at DATABASE_URL, by default
# postgres://loco:loco@localhost:5432/dogfood_test. The tests recreate its schema, so point
# DATABASE_URL at a throwaway database, never one whose data you need.
(cd dogfood && cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test)

# End-to-end: --force-recreate restarts the app, which reseeds the data the tests expect
(cd dogfood && docker compose up --build --force-recreate -d)
until curl -fs http://localhost:5150/_ping >/dev/null; do sleep 2; done
(cd dogfood/e2e && npm ci && npx playwright install chromium && npm test)
```

CI keeps line coverage at or above 90% for the crate and 85% for the frontend.

## The dogfood app

`docker compose up --build` in `dogfood/` serves it at <http://localhost:5150/admin>. Sign in
with password `12341234` as:

- `user1@example.com`: admin
- `editor@example.com`: admin with the Editor role
- `user2@example.com`: no admin access

The database is reseeded on every start; set `SEED_ON_START=false` to keep data.

## Documentation

Setup steps live in `README.md`; everything else is the crate docs in `src/lib.rs`, published on
docs.rs. Preview them with `cargo doc --no-deps --open`.

## Commits and releases

Commit messages and PR titles follow [Conventional Commits](https://www.conventionalcommits.org)
(`feat: ...`, `fix(auth): ...`), and branches are named `<type>/<topic>` (`feat/site-link`).

After CI passes on `main`, release-please keeps a release PR up to date with the next version
and changelog; merging it tags the release and publishes the crate to crates.io. Below 1.0,
`fix:` bumps the patch version, and `feat:` and breaking changes (`feat!:`) the minor. Never
edit the crate version by hand.

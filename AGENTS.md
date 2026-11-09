# AGENTS.md

Admin dashboard crate for Loco apps. `README.md` has the setup steps, the crate docs in `src/lib.rs`
(published on docs.rs) the guide, and `CONTRIBUTING.md` the development workflow.

## Layout

- `src/` crate: `admin.rs` (builder, initializer), `api.rs` (HTTP handlers), `resource.rs`
  (SeaORM operations), `value.rs` (value/JSON conversion), `session.rs` (Loco session sign-in),
  `roles.rs` (roles, their migration and permission checks), `generate.rs` (`admin:models` task),
  `auth.rs`, `meta.rs`, `assets.rs`
- `tests/` crate integration tests; `tests/common` holds test entities and the HTTP client
- `frontend/` React UI; `frontend/dist` is embedded with `rust-embed` and committed
- `dogfood/` Loco app using the crate (Postgres, Docker); `dogfood/e2e/` Playwright tests
- `docs/screenshots/` README and docs.rs screenshots, taken from the dogfood app

## Checks

Run the checks in `CONTRIBUTING.md` before committing; CI runs the same.

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
  (`feat/site-link`, `fix/export-timeout`), not generated or session names. This means that rather than committing to your current branch which you can't rename or delete rather make commits to this other properly named branch.
- Once work is completed and you have created a PR you should run 2 sub agents one using the security review skill and the other using the general code review skill. You should then highlight any validated findings to the user.
- PR descriptions should be concise bullets and in a format in which release please will be able to record each change individually in the changelog.
- Keep `README.md` to setup steps and this file short; document everything else in the crate docs
  (`src/lib.rs`), and keep them in step with code changes.

# Changelog

## [0.3.1](https://github.com/Zesuperaker/muy-loco-admin/compare/v0.3.0...v0.3.1) (2026-11-16)


### Bug Fixes

* **api:** handle a record selected twice once in bulk delete and custom actions, instead of failing with 404 ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))
* **api:** show a foreign key to a column its model hides as a plain field, instead of failing the list and the record with 400 ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))
* **export:** stop a CSV export after Admin::export_timeout in all (10 minutes by default), so a slow client cannot keep a database connection ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))
* **ui:** forms opened from the cache follow the fresh record or role when it arrives, and a role save sends only what was edited ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))
* **ui:** keep integers beyond 2^53 inside JSON fields through the query cache ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))
* **ui:** list filters keep the keyboard focus, and filters the model does not list are shown and can be cleared ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))
* **ui:** mark and describe rejected fields for assistive technology, and reset the page title on pages without one ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))
* **ui:** refuse half-typed dates and unreadable numbers instead of saving them as empty ([2b35d14](https://github.com/Zesuperaker/muy-loco-admin/commit/2b35d14134cbdbc956edb26a386dc69577627724))

## [0.3.0](https://github.com/Zesuperaker/muy-loco-admin/compare/v0.2.0...v0.3.0) (2026-11-11)


### Features

* **list:** add ModelAdmin::cursor_pagination() for large tables, which pages a list from the record before or after each page instead of OFFSET and stops its list and dashboard counts at 10,000 ([9dbd870](https://github.com/Zesuperaker/muy-loco-admin/commit/9dbd870829aa1770eabd0f1eacc7f56c94e086f7))


### Performance Improvements

* **list:** count a list once per search and filters instead of on every page ([9dbd870](https://github.com/Zesuperaker/muy-loco-admin/commit/9dbd870829aa1770eabd0f1eacc7f56c94e086f7))
* **search:** match number, date, UUID and other typed search fields by value, so their indexes serve the search ([9dbd870](https://github.com/Zesuperaker/muy-loco-admin/commit/9dbd870829aa1770eabd0f1eacc7f56c94e086f7))
* **search:** match text search fields with one ILIKE (Postgres) or LIKE (SQLite) that a trigram index can serve, instead of lower(cast(col AS text)) in up to four spellings ([9dbd870](https://github.com/Zesuperaker/muy-loco-admin/commit/9dbd870829aa1770eabd0f1eacc7f56c94e086f7))

## [0.2.0](https://github.com/Zesuperaker/muy-loco-admin/compare/v0.1.1...v0.2.0) (2026-11-09)


### ⚠ BREAKING CHANGES

* `Admin::title` and the `initializers.admin.title` config option are removed; remove `title:` from the admin config, or the admin fails to mount.
* `Admin::router` is now `async`; call it with `.await`.

### Features

* add an admin:models task that lists the app's models ([#15](https://github.com/Zesuperaker/muy-loco-admin/issues/15)) ([45f2948](https://github.com/Zesuperaker/muy-loco-admin/commit/45f2948948f7454a82b63f59897b58df6950ed1e))
* add roles that limit what admin users can do ([#10](https://github.com/Zesuperaker/muy-loco-admin/issues/10)) ([1829981](https://github.com/Zesuperaker/muy-loco-admin/commit/1829981028e9878e0a5ecc13646339805abdaaa5))
* always title the admin "Muy Loco Admin" ([ffc6b7c](https://github.com/Zesuperaker/muy-loco-admin/commit/ffc6b7c6d9acee20d5e850dadd25a87a14a4ad0d))

## [0.1.1](https://github.com/Zesuperaker/muy-loco-admin/compare/v0.1.0...v0.1.1) (2026-11-08)


### Bug Fixes

* **admin:** refuse backslashes in site_url ([d36cbf5](https://github.com/Zesuperaker/muy-loco-admin/commit/d36cbf51806a0c156619fe2559869581d26eb992))
* **api:** stop reporting the crate version in the public config ([d36cbf5](https://github.com/Zesuperaker/muy-loco-admin/commit/d36cbf51806a0c156619fe2559869581d26eb992))
* **auth:** skip tokens that do not validate instead of refusing the request ([d36cbf5](https://github.com/Zesuperaker/muy-loco-admin/commit/d36cbf51806a0c156619fe2559869581d26eb992))
* **session:** refuse Loco's default session secret in release builds ([d36cbf5](https://github.com/Zesuperaker/muy-loco-admin/commit/d36cbf51806a0c156619fe2559869581d26eb992))

## 0.1.0 (2026-11-07)


### Features

* add an admin dashboard for Loco apps ([#2](https://github.com/Zesuperaker/muy-loco-admin/issues/2)) ([d20fe9b](https://github.com/Zesuperaker/muy-loco-admin/commit/d20fe9b59694a203b7daf0e74c4449010fcc4d5c))


### Bug Fixes

* harden security and fix bugs before 0.1.0 ([#4](https://github.com/Zesuperaker/muy-loco-admin/issues/4)) ([8141d31](https://github.com/Zesuperaker/muy-loco-admin/commit/8141d316a1f8136afc3969bf7f617f9dadb7a270))

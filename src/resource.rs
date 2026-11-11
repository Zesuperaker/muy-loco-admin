//! The type-erased model operations behind the admin API.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use axum::body::Bytes;
use futures_util::stream::{self, BoxStream, StreamExt};
use loco_rs::app::AppContext;
use sea_orm::{
    AccessMode, ActiveModelBehavior, ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, EntityName, EntityTrait, IdenStatic,
    Identity, IntoActiveModel, IsolationLevel, Iterable, ModelTrait, Order, PrimaryKeyToColumn,
    PrimaryKeyTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait, Select,
    Statement, TransactionTrait, Value,
    sea_query::{
        Alias, Asterisk, ColumnType, Expr, ExprTrait, Func, IntoIden, LikeExpr, Query, TableName,
        TableRef, ValueTuple,
    },
};
use serde::Serialize;
use serde_json::{Map, Value as Json};

use crate::{
    error::{AdminError, AdminResult, FieldErrors},
    meta::{
        ActionMeta, Capabilities, FieldKind, FieldMeta, ModelMeta, Ordering, Pagination,
        RelationMeta, SortDirection, enum_choices, humanize, max_length,
    },
    model::{Action, ModelAdmin},
    query::{Cursor, ListParams, NOT_NULL_FILTER, NULL_FILTER},
    value::{convert, decode_key, encode_key, is_null, json_to_text, to_json, to_text},
};

const DEFAULT_PER_PAGE: u64 = 25;
const EXPORT_BATCH: u64 = 500;
/// With cursor pagination, counting stops after this many records, so a count
/// never reads more.
pub(crate) const COUNT_LIMIT: u64 = 10_000;
const SENSITIVE: &[&str] = &["password", "secret", "token", "api_key", "salt"];
/// Loco's timestamp columns: read-only by default and listed after the other fields.
const TIMESTAMPS: &[&str] = &["created_at", "updated_at"];
const DISPLAY_CANDIDATES: &[&str] = &[
    "name", "title", "label", "email", "username", "slug", "code",
];

/// One record as sent to the UI.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "AdminRecord"))]
pub struct Record {
    /// Encoded primary key, used in URLs.
    pub key: String,
    /// Human-readable label.
    pub label: String,
    /// Column name to value.
    pub values: Map<String, Json>,
}

/// One page of a list.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[allow(clippy::struct_field_names)]
pub struct Page {
    pub records: Vec<Record>,
    pub per_page: u64,
    /// The page's number, when the list has numbered pages.
    pub page: Option<u64>,
    /// What fetches the next page, if there is one: its number, or with
    /// cursor pagination its `after` cursor.
    pub next: Option<String>,
    /// What fetches the previous page, if there is one: its number, or with
    /// cursor pagination its `before` cursor.
    pub prev: Option<String>,
}

/// How many records a list has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Count {
    /// Exactly this many.
    Exact(u64),
    /// More than this many: counting stopped at [`COUNT_LIMIT`].
    MoreThan(u64),
    /// About this many, from Postgres's table statistics.
    Estimate(u64),
}

/// An option offered by a foreign-key picker.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Choice {
    pub key: String,
    pub value: Json,
    pub label: String,
}

/// A CSV export: the header, then one chunk per batch of rows.
pub(crate) type CsvStream = BoxStream<'static, AdminResult<Bytes>>;

/// Model operations used by the HTTP layer.
#[async_trait]
pub(crate) trait Resource: Send + Sync {
    fn meta(&self) -> &ModelMeta;
    /// The records matching `params`' search and filters: exact, or up to
    /// [`COUNT_LIMIT`] for a model with cursor pagination.
    async fn count(&self, db: &DatabaseConnection, params: &ListParams) -> AdminResult<Count>;
    /// A page of the records matching `params`: page `params.page`, or with
    /// cursor pagination the page next to `params.cursor`.
    async fn list(&self, db: &DatabaseConnection, params: &ListParams) -> AdminResult<Page>;
    async fn get(&self, db: &DatabaseConnection, key: &str) -> AdminResult<Record>;
    /// Insert a record from `input`. Fields in `locked` are read-only, like
    /// the model's own read-only fields.
    async fn create(
        &self,
        db: &DatabaseConnection,
        input: Map<String, Json>,
        locked: &[String],
    ) -> AdminResult<Record>;
    /// Change a record with `input`. Fields in `locked` are read-only, like
    /// the model's own read-only fields.
    async fn update(
        &self,
        db: &DatabaseConnection,
        key: &str,
        input: Map<String, Json>,
        locked: &[String],
    ) -> AdminResult<Record>;
    async fn delete(&self, db: &DatabaseConnection, keys: &[String]) -> AdminResult<u64>;
    /// Stream the filtered records as CSV, in primary-key order. The database
    /// ends the export's transaction if it stays idle for `idle_limit`.
    async fn export(
        self: Arc<Self>,
        db: &DatabaseConnection,
        params: &ListParams,
        idle_limit: Duration,
    ) -> AdminResult<CsvStream>;
    /// Up to `limit` choices for a foreign-key picker: records whose search
    /// fields match `q`, or whose `column` (the key by default) starts with
    /// it or, if it is not text, equals it.
    async fn lookup(
        &self,
        db: &DatabaseConnection,
        q: Option<&str>,
        column: Option<&str>,
        limit: u64,
    ) -> AdminResult<Vec<Choice>>;
    async fn labels(
        &self,
        db: &DatabaseConnection,
        column: &str,
        values: &[Json],
    ) -> AdminResult<BTreeMap<String, String>>;
    async fn run_action(
        &self,
        ctx: &AppContext,
        name: &str,
        keys: &[String],
    ) -> AdminResult<String>;
}

struct Column<E: EntityTrait> {
    col: E::Column,
    name: String,
    kind: FieldKind,
    readonly: bool,
    primary_key: bool,
}

pub(crate) struct ModelResource<E: EntityTrait> {
    meta: ModelMeta,
    columns: Vec<Column<E>>,
    pk: Vec<E::Column>,
    search: Vec<usize>,
    display: Option<E::Column>,
    order: (E::Column, SortDirection),
    actions: Vec<Action<E>>,
}

fn table_of(table: &TableRef) -> Option<String> {
    match table {
        TableRef::Table(TableName(_, name), _) => Some(name.to_string()),
        _ => None,
    }
}

fn single_column(identity: &sea_orm::Identity) -> Option<String> {
    match identity {
        sea_orm::Identity::Unary(col) => Some(col.to_string()),
        _ => None,
    }
}

/// Whether a column name looks like it holds a credential.
pub(crate) fn is_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE.iter().any(|s| lower.contains(s))
}

/// Spellings of `q` to look for. Searches ignore the case of ASCII letters,
/// but SQLite's `LIKE`, and Postgres's `ILIKE` under the `C` locale, match
/// other letters only in the case they are typed in: a `q` with such letters
/// is also tried in lower, upper and title case.
fn search_terms(q: &str) -> Vec<String> {
    if !q
        .chars()
        .any(|c| !c.is_ascii() && (c.is_lowercase() || c.is_uppercase()))
    {
        return vec![q.to_string()];
    }
    let mut word_start = true;
    let title: String = q
        .chars()
        .flat_map(|c| {
            let cased: Vec<char> = if word_start {
                c.to_uppercase().collect()
            } else {
                c.to_lowercase().collect()
            };
            word_start = !c.is_alphanumeric();
            cased
        })
        .collect();
    let mut terms: Vec<String> = [q.to_string(), q.to_lowercase(), q.to_uppercase(), title]
        .iter()
        .map(|term| term.to_ascii_lowercase())
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

/// `expr` matching the `LIKE` `pattern`, whose wildcards [`escape_like`]
/// escapes, ignoring the case of ASCII letters at least: `ILIKE` on Postgres,
/// which a trigram index on the column serves, and `LIKE` on SQLite, which
/// ignores ASCII case itself.
fn like(backend: DbBackend, expr: Expr, pattern: String) -> Expr {
    use sea_orm::sea_query::extension::postgres::PgExpr;
    if backend == DbBackend::Postgres {
        // Backslash is Postgres's default escape character, and sea-query
        // writes `ILIKE` with an `ESCAPE` clause in invalid brackets.
        expr.ilike(LikeExpr::new(pattern))
    } else {
        expr.like(LikeExpr::new(pattern).escape('\\'))
    }
}

/// Postgres's estimate of the rows in `entity`'s table, from the statistics
/// `ANALYZE` and autovacuum keep, scaled to the table's current size as the
/// query planner does. `None` before the table is first analyzed.
async fn estimate(db: &DatabaseConnection, entity: &impl EntityName) -> Result<Option<u64>, DbErr> {
    let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let table = match entity.schema_name() {
        Some(schema) => format!("{}.{}", quote(schema), quote(entity.table_name())),
        None => quote(entity.table_name()),
    };
    let sql = "SELECT (reltuples::float8 / relpages \
               * (pg_relation_size(oid) / current_setting('block_size')::float8))::bigint \
               FROM pg_class WHERE oid = to_regclass($1) AND reltuples >= 0 AND relpages > 0";
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            [table.into()],
        ))
        .await?;
    Ok(row
        .map(|row| row.try_get_by_index::<i64>(0))
        .transpose()?
        .and_then(|n| u64::try_from(n).ok()))
}

fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Neutralize spreadsheet formulas in exported text cells.
fn csv_cell(kind: FieldKind, text: String) -> String {
    let numeric = matches!(
        kind,
        FieldKind::Integer | FieldKind::Float | FieldKind::Decimal
    );
    if !numeric && text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text
    }
}

fn csv_chunk<R>(rows: impl IntoIterator<Item = R>) -> AdminResult<Bytes>
where
    R: IntoIterator,
    R::Item: AsRef<[u8]>,
{
    let internal = |e: &dyn std::fmt::Display| AdminError::Internal(e.to_string());
    let mut writer = csv::Writer::from_writer(Vec::new());
    for row in rows {
        writer.write_record(row).map_err(|e| internal(&e))?;
    }
    writer
        .into_inner()
        .map(Bytes::from)
        .map_err(|e| internal(&e))
}

/// A read-only transaction over one snapshot of the data, so rows changed
/// during an export are neither skipped nor repeated. SQLite read
/// transactions are snapshots already and take no options.
///
/// On Postgres the transaction is also ended by the server if it sits idle
/// for `idle_limit`, as a backstop for the export's own idle timeout.
async fn snapshot(
    db: &DatabaseConnection,
    idle_limit: Duration,
) -> Result<DatabaseTransaction, DbErr> {
    if db.get_database_backend() != DbBackend::Postgres {
        return db.begin().await;
    }
    let txn = db
        .begin_with_config(
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadOnly),
        )
        .await?;
    txn.execute_unprepared(&format!(
        "SET LOCAL idle_in_transaction_session_timeout = {}",
        idle_limit.as_millis()
    ))
    .await?;
    Ok(txn)
}

/// Empty strings mean "no value" for non-text fields.
fn normalize(kind: FieldKind, input: Json) -> Json {
    match &input {
        Json::String(s) if s.trim().is_empty() && !kind.is_textual() => Json::Null,
        _ => input,
    }
}

/// A list's order: a column, then the rest of the primary key to break ties.
struct Sort<E: EntityTrait> {
    col: E::Column,
    /// Postgres's `json` type has no ordering, so JSON sorts by its text.
    json: bool,
    nullable: bool,
    asc: bool,
}

impl<E: EntityTrait> Sort<E> {
    /// What the list is ordered by.
    fn expr(&self) -> Expr {
        let expr = Expr::col((E::default(), self.col));
        if self.json {
            expr.cast_as(Alias::new("text"))
        } else {
            expr
        }
    }

    /// The direction of this order.
    fn order(&self) -> Order {
        if self.asc { Order::Asc } else { Order::Desc }
    }

    /// The same order, backwards.
    fn reversed(&self) -> Self {
        Self {
            col: self.col,
            json: self.json,
            nullable: self.nullable,
            asc: !self.asc,
        }
    }

    /// Whether `NULL`s come first. Postgres sorts them as larger than any
    /// value and SQLite as smaller; lists keep that order, so the database
    /// can read them from an index.
    fn nulls_first(&self, backend: DbBackend) -> bool {
        (backend == DbBackend::Postgres) != self.asc
    }

    /// `a` comes after `b` in this order, or level with it if `inclusive`.
    fn later(&self, a: Expr, b: Expr, inclusive: bool) -> Expr {
        match (self.asc, inclusive) {
            (true, false) => a.gt(b),
            (true, true) => a.gte(b),
            (false, false) => a.lt(b),
            (false, true) => a.lte(b),
        }
    }
}

/// The record a page starts from: the condition that finds it, and whether
/// its sort value is `NULL`. Pages compare the other records with its stored
/// values in SQL: values read into Rust and bound again may not compare the
/// same, such as SQLite timestamps that `CURRENT_TIMESTAMP` wrote and sqlx
/// writes differently, or Postgres's space-padded `char(n)`.
struct Anchor {
    key: Condition,
    null: bool,
}

impl<E> ModelResource<E>
where
    E: EntityTrait,
{
    #[allow(clippy::too_many_lines)]
    pub(crate) fn new(config: ModelAdmin<E>) -> Result<Self, String> {
        let table = E::default().table_name().to_string();
        let name = config.name.clone().unwrap_or_else(|| table.clone());
        let pk: Vec<E::Column> = E::PrimaryKey::iter()
            .map(PrimaryKeyToColumn::into_column)
            .collect();
        let pk_names: Vec<String> = pk.iter().map(|c| c.as_str().to_string()).collect();
        let auto_increment = <E::PrimaryKey as PrimaryKeyTrait>::auto_increment();

        // Primary key first and timestamps last, so the list and form open on the
        // columns that identify and describe a record rather than on `created_at`.
        let mut all: Vec<E::Column> = E::Column::iter().collect();
        all.sort_by_key(|c| {
            let name = c.as_str();
            if pk_names.iter().any(|k| k == name) {
                0
            } else if TIMESTAMPS.contains(&name) {
                2
            } else {
                1
            }
        });
        let all_names: Vec<String> = all.iter().map(|c| c.as_str().to_string()).collect();
        let check = |what: &str, list: &[String]| -> Result<(), String> {
            for field in list {
                if !all_names.contains(field) {
                    return Err(format!("model `{name}`: unknown field `{field}` in {what}"));
                }
            }
            Ok(())
        };

        // Sensitive-looking columns stay hidden whatever `hidden_fields` lists;
        // only `show_fields` reveals one.
        check("hidden_fields", &config.hidden_fields)?;
        check("show_fields", &config.show_fields)?;
        if let Some(field) = config
            .hidden_fields
            .iter()
            .find(|f| config.show_fields.contains(*f))
        {
            return Err(format!(
                "model `{name}`: field `{field}` is in both hidden_fields and show_fields"
            ));
        }
        let hidden: HashSet<String> = all_names
            .iter()
            .filter(|n| is_sensitive(n) && !config.show_fields.contains(*n))
            .chain(&config.hidden_fields)
            .cloned()
            .collect();
        if let Some(key) = pk_names.iter().find(|k| hidden.contains(*k)) {
            let hint = if config.hidden_fields.contains(key) {
                ""
            } else {
                " (its name looks sensitive; list it in show_fields to show it)"
            };
            return Err(format!(
                "model `{name}`: primary key `{key}` cannot be hidden{hint}"
            ));
        }
        let readonly: HashSet<String> = match &config.readonly_fields {
            Some(list) => {
                check("readonly_fields", list)?;
                list.iter().cloned().collect()
            }
            None => TIMESTAMPS.iter().map(ToString::to_string).collect(),
        };

        let foreign_keys: BTreeMap<String, (String, String)> = E::Relation::iter()
            .filter_map(|rel| {
                let def = rel.def();
                if def.is_owner || table_of(&def.from_tbl)? != table {
                    return None;
                }
                Some((
                    single_column(&def.from_col)?,
                    (table_of(&def.to_tbl)?, single_column(&def.to_col)?),
                ))
            })
            .collect();

        let mut columns = Vec::new();
        let mut fields = Vec::new();
        for col in &all {
            let col_name = col.as_str().to_string();
            if hidden.contains(&col_name) {
                continue;
            }
            let def = col.def();
            let col_type = def.get_column_type().clone();
            let kind = FieldKind::from_column_type(&col_type);
            let primary_key = pk_names.contains(&col_name);
            let auto = primary_key && auto_increment;
            let is_readonly = auto || kind.is_read_only() || readonly.contains(&col_name);
            fields.push(FieldMeta {
                label: humanize(&col_name),
                name: col_name.clone(),
                kind,
                nullable: def.is_null(),
                primary_key,
                auto_increment: auto,
                readonly: is_readonly,
                unique: def.is_unique() || (primary_key && pk_names.len() == 1),
                timezone: matches!(col_type, ColumnType::TimestampWithTimeZone),
                max_length: max_length(&col_type),
                choices: enum_choices(&col_type),
                relation: foreign_keys.get(&col_name).map(|(t, c)| RelationMeta {
                    table: t.clone(),
                    column: c.clone(),
                    model: None,
                }),
                role: false,
            });
            columns.push(Column {
                col: *col,
                name: col_name,
                kind,
                readonly: is_readonly,
                primary_key,
            });
        }

        let visible = |what: &str, list: &[String]| -> Result<(), String> {
            check(what, list)?;
            for field in list {
                if hidden.contains(field) {
                    return Err(format!(
                        "model `{name}`: hidden field `{field}` used in {what}"
                    ));
                }
            }
            Ok(())
        };
        let list_display = match config.list_display {
            Some(list) => {
                visible("list_display", &list)?;
                list
            }
            None => fields
                .iter()
                .filter(|f| {
                    !matches!(
                        f.kind,
                        FieldKind::Text | FieldKind::Json | FieldKind::Binary | FieldKind::Array
                    )
                })
                .map(|f| f.name.clone())
                .collect(),
        };
        let search_fields = match config.search_fields {
            Some(list) => {
                visible("search_fields", &list)?;
                list
            }
            None => fields
                .iter()
                .filter(|f| f.kind.is_textual())
                .map(|f| f.name.clone())
                .collect(),
        };
        let list_filter = match config.list_filter {
            Some(list) => {
                visible("list_filter", &list)?;
                if let Some(field) = list.iter().find(|name| {
                    fields
                        .iter()
                        .any(|f| &f.name == *name && f.kind == FieldKind::Json)
                }) {
                    return Err(format!(
                        "model `{name}`: JSON field `{field}` cannot be used in list_filter"
                    ));
                }
                list
            }
            None => fields
                .iter()
                .filter(|f| {
                    matches!(f.kind, FieldKind::Boolean | FieldKind::Enum) || f.relation.is_some()
                })
                .map(|f| f.name.clone())
                .collect(),
        };
        let (order_name, direction) = match config.ordering {
            Some((field, direction)) => {
                visible("ordering", std::slice::from_ref(&field))?;
                (field, direction)
            }
            None => (pk_names[0].clone(), SortDirection::Desc),
        };
        let display_field = match config.display_field {
            Some(field) => {
                visible("display_field", std::slice::from_ref(&field))?;
                Some(field)
            }
            None => DISPLAY_CANDIDATES
                .iter()
                .find(|c| fields.iter().any(|f| f.name == **c && f.kind.is_textual()))
                .map(ToString::to_string),
        };

        let find = |field: &str| columns.iter().position(|c| c.name == field);
        let search = search_fields.iter().filter_map(|f| find(f)).collect();
        let display = display_field
            .as_deref()
            .and_then(find)
            .map(|i| columns[i].col);
        let order_col = find(&order_name).map_or(pk[0], |i| columns[i].col);

        let mut seen = HashSet::new();
        for action in &config.actions {
            if !seen.insert(action.name.clone()) {
                return Err(format!(
                    "model `{name}`: duplicate action `{}`",
                    action.name
                ));
            }
        }
        let editable = columns.iter().any(|c| !c.readonly && !c.primary_key);
        let meta = ModelMeta {
            label: config.label.clone().unwrap_or_else(|| humanize(&table)),
            name,
            table,
            primary_key: pk_names,
            fields,
            list_display,
            search_fields,
            list_filter,
            ordering: Ordering {
                field: order_name,
                direction,
            },
            per_page: config
                .per_page
                .unwrap_or(DEFAULT_PER_PAGE)
                .clamp(1, crate::query::MAX_PER_PAGE),
            pagination: if config.cursor_pagination {
                Pagination::Cursor
            } else {
                Pagination::Pages
            },
            display_field,
            actions: config
                .actions
                .iter()
                .map(|a| ActionMeta {
                    name: a.name.clone(),
                    label: a.label.clone(),
                })
                .collect(),
            related: Vec::new(),
            capabilities: Capabilities {
                view: true,
                create: config.can_create,
                update: config.can_update && editable,
                delete: config.can_delete,
                export: config.can_export,
            },
        };

        Ok(Self {
            meta,
            columns,
            pk,
            search,
            display,
            order: (order_col, direction),
            actions: config.actions,
        })
    }

    fn column(&self, name: &str) -> AdminResult<&Column<E>> {
        self.columns
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| AdminError::bad_request(format!("unknown field `{name}`")))
    }

    fn convert(column: &Column<E>, input: Json) -> Result<Value, String> {
        convert::<E::ActiveModel>(column.col, &normalize(column.kind, input))
    }

    fn record(&self, model: &E::Model) -> Record {
        let key = encode_key(&self.pk.iter().map(|c| model.get(*c)).collect::<Vec<_>>());
        let values = self
            .columns
            .iter()
            .map(|c| (c.name.clone(), to_json(&model.get(c.col))))
            .collect();
        Record {
            label: self.label(model, &key),
            key,
            values,
        }
    }

    fn label(&self, model: &E::Model, key: &str) -> String {
        self.display
            .map(|c| to_text(&model.get(c)))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("#{key}"))
    }

    fn key_condition(&self, key: &str) -> AdminResult<Condition> {
        let parts = decode_key(key);
        if parts.len() != self.pk.len() {
            return Err(AdminError::NotFound);
        }
        let mut cond = Condition::all();
        for (col, part) in self.pk.iter().zip(parts) {
            let value = convert::<E::ActiveModel>(*col, &Json::String(part))
                .map_err(|_| AdminError::NotFound)?;
            cond = cond.add(col.eq(value));
        }
        Ok(cond)
    }

    async fn find<C: ConnectionTrait>(&self, db: &C, key: &str) -> AdminResult<E::Model> {
        E::find()
            .filter(self.key_condition(key)?)
            .one(db)
            .await?
            .ok_or(AdminError::NotFound)
    }

    /// Whether `params` narrows the list down.
    fn filtered(&self, params: &ListParams) -> bool {
        !params.filters.is_empty() || (params.q.is_some() && !self.search.is_empty())
    }

    /// The records matching `params`' search and filters, in no order.
    fn query(&self, backend: DbBackend, params: &ListParams) -> AdminResult<Select<E>> {
        let mut cond = Condition::all();
        if let Some(q) = &params.q
            && !self.search.is_empty()
        {
            cond = cond.add(self.search_condition(backend, q));
        }
        let mut errors = FieldErrors::new();
        for (field, raw) in &params.filters {
            let column = self.column(field)?;
            if column.kind == FieldKind::Json {
                return Err(AdminError::bad_request(format!(
                    "cannot filter by the JSON field `{field}`"
                )));
            }
            match raw.as_str() {
                NULL_FILTER => cond = cond.add(column.col.is_null()),
                NOT_NULL_FILTER => cond = cond.add(column.col.is_not_null()),
                _ => match Self::convert(column, Json::String(raw.clone())) {
                    Ok(value) if !is_null(&value) => cond = cond.add(column.col.eq(value)),
                    Ok(_) => cond = cond.add(column.col.is_null()),
                    Err(msg) => {
                        errors.entry(field.clone()).or_default().push(msg);
                    }
                },
            }
        }
        if !errors.is_empty() {
            return Err(AdminError::Validation {
                message: "invalid filter".to_string(),
                fields: errors,
            });
        }
        Ok(E::find().filter(cond))
    }

    /// Records whose search fields match `q`. Text fields contain it, ignoring
    /// case. JSON, binary, array and other fields contain it in their text,
    /// which no index serves. The rest, such as numbers, dates and UUIDs, equal
    /// it when it is one of their values, so their indexes serve the search.
    fn search_condition(&self, backend: DbBackend, q: &str) -> Condition {
        let terms = search_terms(q);
        let mut any = Condition::any();
        for column in self.search.iter().map(|i| &self.columns[*i]) {
            let expr = Expr::col((E::default(), column.col));
            let text = match column.kind {
                FieldKind::String | FieldKind::Text => expr,
                FieldKind::Json | FieldKind::Binary | FieldKind::Array | FieldKind::Other => {
                    expr.cast_as(Alias::new("text"))
                }
                _ => {
                    any = any.add_option(Self::exact(column, q).map(|v| column.col.eq(v)));
                    continue;
                }
            };
            for term in &terms {
                any = any.add(like(
                    backend,
                    text.clone(),
                    format!("%{}%", escape_like(term)),
                ));
            }
        }
        any
    }

    /// `q` as a value of `column`, if it is one.
    fn exact(column: &Column<E>, q: &str) -> Option<Value> {
        Self::convert(column, Json::String(q.to_string()))
            .ok()
            .filter(|value| !is_null(value))
    }

    /// The order `params` asks for: its `sort` column (ascending unless
    /// `order` says otherwise), or the model's default ordering.
    fn sort(&self, params: &ListParams) -> AdminResult<Sort<E>> {
        let (col, default_dir) = match &params.sort {
            Some(field) => (self.column(field)?.col, SortDirection::Asc),
            None => self.order,
        };
        Ok(Sort {
            col,
            json: self
                .columns
                .iter()
                .any(|c| c.col.as_str() == col.as_str() && c.kind == FieldKind::Json),
            nullable: col.def().is_null(),
            asc: params.order.unwrap_or(default_dir) == SortDirection::Asc,
        })
    }

    /// The key columns that break ties in `sort` order.
    fn tiebreak<'a>(&'a self, sort: &'a Sort<E>) -> impl Iterator<Item = E::Column> + 'a {
        self.pk
            .iter()
            .copied()
            .filter(|pk| pk.as_str() != sort.col.as_str())
    }

    /// `select` in `sort` order, with the rest of the key breaking ties so the
    /// order is the same on every request.
    fn ordered(&self, select: Select<E>, sort: &Sort<E>) -> Select<E> {
        let mut select = select.order_by(sort.expr(), sort.order());
        for pk in self.tiebreak(sort) {
            select = select.order_by(pk, sort.order());
        }
        select
    }

    /// The record with `key` as a page's starting point, or `None` if there
    /// is no such record.
    async fn anchor(
        &self,
        db: &DatabaseConnection,
        sort: &Sort<E>,
        key: &str,
    ) -> AdminResult<Option<Anchor>> {
        let Ok(key) = self.key_condition(key) else {
            return Ok(None);
        };
        let Some(model) = E::find().filter(key.clone()).one(db).await? else {
            return Ok(None);
        };
        let value = model.get(sort.col);
        Ok(Some(Anchor {
            null: value == value.as_null(),
            key,
        }))
    }

    /// The records after `anchor` in `sort` order, or level with it too if
    /// `inclusive`, as conditions on consecutive runs of the list: records
    /// with a sort value, and those without, in the order the database sorts
    /// `NULL`s. Each compares the sort value and key as a row with the
    /// anchor's, which an index on them serves; joined with `OR`, they would
    /// not be one index range.
    fn after(
        &self,
        backend: DbBackend,
        sort: &Sort<E>,
        anchor: &Anchor,
        inclusive: bool,
    ) -> Vec<Expr> {
        // The same columns of the anchor's stored row.
        let stored = |exprs: &[Expr]| {
            let mut select = Query::select();
            select
                .exprs(exprs.iter().cloned())
                .from(E::default())
                .cond_where(anchor.key.clone());
            Expr::SubQuery(None, Box::new(select.into()))
        };
        let keys: Vec<Expr> = self
            .tiebreak(sort)
            .map(|c| Expr::col((E::default(), c)))
            .collect();
        let sorted = sort.expr();
        if anchor.null {
            // Key columns are never NULL, so the sort column is not one, and
            // the key alone orders the records with no sort value.
            let nulls = sorted.clone().is_null().and(sort.later(
                Expr::tuple(keys.clone()),
                stored(&keys),
                inclusive,
            ));
            if sort.nulls_first(backend) {
                vec![nulls, sorted.is_not_null()]
            } else {
                vec![nulls]
            }
        } else {
            let mut row = vec![sorted.clone()];
            row.extend(keys);
            let values = sort.later(Expr::tuple(row.clone()), stored(&row), inclusive);
            if sort.nullable && !sort.nulls_first(backend) {
                vec![values, sorted.is_null()]
            } else {
                vec![values]
            }
        }
    }

    /// Up to `limit` records of `query` in `sort` order, after `from` (or
    /// from it, if inclusive) when given.
    async fn fetch(
        &self,
        db: &DatabaseConnection,
        query: Select<E>,
        sort: &Sort<E>,
        from: Option<(&Anchor, bool)>,
        limit: u64,
    ) -> AdminResult<Vec<E::Model>> {
        let Some((anchor, inclusive)) = from else {
            return Ok(self.ordered(query, sort).limit(limit).all(db).await?);
        };
        let mut models = Vec::new();
        for run in self.after(db.get_database_backend(), sort, anchor, inclusive) {
            let missing = limit.saturating_sub(models.len() as u64);
            if missing == 0 {
                break;
            }
            let select = self.ordered(query.clone().filter(run), sort);
            models.extend(select.limit(missing).all(db).await?);
        }
        Ok(models)
    }

    /// A page of `models` with cursor pagination: its first record is the
    /// cursor to the previous page if `prev`, and its last to the next if
    /// `next`.
    fn page(&self, models: &[E::Model], per_page: u64, prev: bool, next: bool) -> Page {
        let records: Vec<Record> = models.iter().map(|m| self.record(m)).collect();
        Page {
            prev: records.first().filter(|_| prev).map(|r| r.key.clone()),
            next: records.last().filter(|_| next).map(|r| r.key.clone()),
            records,
            per_page,
            page: None,
        }
    }

    /// Page `page` of `query` in `sort` order, which skips the records of the
    /// pages before it.
    async fn numbered_page(
        &self,
        db: &DatabaseConnection,
        query: Select<E>,
        sort: &Sort<E>,
        page: u64,
        per_page: u64,
    ) -> AdminResult<Page> {
        let page = Ord::max(page, 1);
        // Databases take a signed 64-bit offset.
        let offset = (page - 1)
            .checked_mul(per_page)
            .filter(|offset| i64::try_from(*offset).is_ok())
            .ok_or_else(|| AdminError::bad_request("`page` is too large"))?;
        let mut models = self
            .ordered(query, sort)
            .offset(offset)
            .limit(per_page + 1)
            .all(db)
            .await?;
        let more = truncate(&mut models, per_page);
        Ok(Page {
            records: models.iter().map(|m| self.record(m)).collect(),
            per_page,
            page: Some(page),
            next: more.then(|| (page + 1).to_string()),
            prev: (page > 1).then(|| (page - 1).to_string()),
        })
    }

    /// How many records match `params`, stopping after `limit` if given.
    async fn count_up_to(
        &self,
        db: &DatabaseConnection,
        params: &ListParams,
        limit: Option<u64>,
    ) -> AdminResult<Count> {
        let backend = db.get_database_backend();
        let mut matches = self
            .query(backend, params)?
            .select_only()
            .column(self.pk[0]);
        if let Some(limit) = limit {
            matches = matches.limit(limit + 1);
        }
        let matches = matches.into_query();
        let stmt = Query::select()
            .expr(Func::count(Expr::col(Asterisk)))
            .from_subquery(matches, Alias::new("matches"))
            .to_owned();
        let counted: i64 = match db.query_one(&stmt).await? {
            Some(row) => row.try_get_by_index(0)?,
            None => 0,
        };
        let counted = u64::try_from(counted).unwrap_or_default();
        let Some(limit) = limit.filter(|limit| counted > *limit) else {
            return Ok(Count::Exact(counted));
        };
        if backend == DbBackend::Postgres
            && !self.filtered(params)
            && let Some(estimate) = estimate(db, &E::default()).await?
            && estimate > limit
        {
            return Ok(Count::Estimate(estimate));
        }
        Ok(Count::MoreThan(limit))
    }

    /// The first page of `query` in `sort` order, with cursor pagination.
    async fn first_page(
        &self,
        db: &DatabaseConnection,
        query: Select<E>,
        sort: &Sort<E>,
        per_page: u64,
    ) -> AdminResult<Page> {
        let mut models = self.fetch(db, query, sort, None, per_page + 1).await?;
        let more = truncate(&mut models, per_page);
        Ok(self.page(&models, per_page, false, more))
    }

    /// The page that ends just before `anchor`, or with it if `inclusive`.
    async fn page_before(
        &self,
        db: &DatabaseConnection,
        query: Select<E>,
        sort: &Sort<E>,
        anchor: &Anchor,
        inclusive: bool,
        per_page: u64,
    ) -> AdminResult<Page> {
        let backwards = sort.reversed();
        let mut models = self
            .fetch(
                db,
                query.clone(),
                &backwards,
                Some((anchor, inclusive)),
                per_page + 1,
            )
            .await?;
        if !truncate(&mut models, per_page) {
            // These reach the start of the list: show a full first page.
            return self.first_page(db, query, sort, per_page).await;
        }
        models.reverse();
        Ok(self.page(&models, per_page, true, !inclusive))
    }

    fn apply_input(
        &self,
        am: &mut E::ActiveModel,
        current: Option<&E::Model>,
        input: Map<String, Json>,
        locked: &[String],
    ) -> AdminResult<bool> {
        let mut errors = FieldErrors::new();
        let mut changed = false;
        for (name, value) in input {
            // Hidden columns get the same answer as missing ones, so the reply
            // does not reveal that they exist.
            let Some(column) = self.columns.iter().find(|c| c.name == name) else {
                errors
                    .entry(name)
                    .or_default()
                    .push("is not a field of this model".to_string());
                continue;
            };
            if column.readonly
                || locked.contains(&name)
                || (current.is_some() && column.primary_key)
            {
                errors
                    .entry(name)
                    .or_default()
                    .push("is read-only".to_string());
                continue;
            }
            match Self::convert(column, value) {
                Ok(value) => {
                    if current.is_some_and(|m| m.get(column.col) == value) {
                        continue;
                    }
                    match am.try_set(column.col, value) {
                        Ok(()) => changed = true,
                        Err(err) => {
                            tracing::info!(field = %name, error = %err, "admin field rejected");
                            errors
                                .entry(name)
                                .or_default()
                                .push("has an invalid value".to_string());
                        }
                    }
                }
                Err(msg) => errors.entry(name).or_default().push(msg),
            }
        }
        if errors.is_empty() {
            Ok(changed)
        } else {
            Err(AdminError::invalid(errors))
        }
    }
}

/// A running export, read in keyset-paginated batches inside one snapshot.
struct Export<E: EntityTrait> {
    resource: Arc<ModelResource<E>>,
    query: Select<E>,
    txn: Option<DatabaseTransaction>,
    /// Primary key of the last exported row.
    after: Option<Vec<Value>>,
}

impl<E> Export<E>
where
    E: EntityTrait + Sync,
    E::Model: Sync,
{
    /// The next batch as CSV rows, or `None` when every row was sent.
    async fn next(mut self) -> AdminResult<Option<(Bytes, Self)>> {
        let models = {
            let Some(txn) = self.txn.as_ref() else {
                return Ok(None);
            };
            let pk = self.resource.pk.iter().map(|c| c.into_iden()).collect();
            let mut cursor = self.query.clone().cursor_by(Identity::Many(pk));
            cursor.first(EXPORT_BATCH);
            if let Some(after) = self.after.take() {
                cursor.after(ValueTuple::Many(after));
            }
            cursor.all(txn).await?
        };
        if (models.len() as u64) < EXPORT_BATCH
            && let Some(txn) = self.txn.take()
        {
            txn.commit().await?;
        }
        let Some(last) = models.last() else {
            return Ok(None);
        };
        self.after = Some(self.resource.pk.iter().map(|c| last.get(*c)).collect());
        let chunk = csv_chunk(models.iter().map(|model| {
            self.resource
                .columns
                .iter()
                .map(|c| csv_cell(c.kind, to_text(&model.get(c.col))))
        }))?;
        Ok(Some((chunk, self)))
    }
}

#[async_trait]
impl<E> Resource for ModelResource<E>
where
    E: EntityTrait + Sync,
    E::Model: IntoActiveModel<E::ActiveModel> + Sync,
    E::ActiveModel: ActiveModelBehavior + Send + Sync,
{
    fn meta(&self) -> &ModelMeta {
        &self.meta
    }

    /// Counts every record, unless the model has cursor pagination, which
    /// stops counting at [`COUNT_LIMIT`].
    async fn count(&self, db: &DatabaseConnection, params: &ListParams) -> AdminResult<Count> {
        let limit = (self.meta.pagination == Pagination::Cursor).then_some(COUNT_LIMIT);
        self.count_up_to(db, params, limit).await
    }

    /// Numbered pages skip the pages before them; cursor pagination starts
    /// from the cursor's record, and starts over if it is gone.
    async fn list(&self, db: &DatabaseConnection, params: &ListParams) -> AdminResult<Page> {
        let per_page = params.per_page.unwrap_or(self.meta.per_page);
        let sort = self.sort(params)?;
        let query = self.query(db.get_database_backend(), params)?;
        if self.meta.pagination == Pagination::Pages {
            return self
                .numbered_page(db, query, &sort, params.page, per_page)
                .await;
        }
        let (key, forward) = match &params.cursor {
            Some(Cursor::After(key)) => (key, true),
            Some(Cursor::Before(key)) => (key, false),
            None => return self.first_page(db, query, &sort, per_page).await,
        };
        // A cursor whose record is gone starts the list over.
        let Some(anchor) = self.anchor(db, &sort, key).await? else {
            return self.first_page(db, query, &sort, per_page).await;
        };
        if !forward {
            return self
                .page_before(db, query, &sort, &anchor, false, per_page)
                .await;
        }
        let mut models = self
            .fetch(
                db,
                query.clone(),
                &sort,
                Some((&anchor, false)),
                per_page + 1,
            )
            .await?;
        if models.is_empty() {
            // The records after the cursor are gone: show the page ending with it.
            return self
                .page_before(db, query, &sort, &anchor, true, per_page)
                .await;
        }
        let more = truncate(&mut models, per_page);
        Ok(self.page(&models, per_page, true, more))
    }

    async fn get(&self, db: &DatabaseConnection, key: &str) -> AdminResult<Record> {
        Ok(self.record(&self.find(db, key).await?))
    }

    async fn create(
        &self,
        db: &DatabaseConnection,
        input: Map<String, Json>,
        locked: &[String],
    ) -> AdminResult<Record> {
        let mut am = <E::ActiveModel as ActiveModelBehavior>::new();
        self.apply_input(&mut am, None, input, locked)?;
        let model = am.insert(db).await?;
        Ok(self.record(&model))
    }

    async fn update(
        &self,
        db: &DatabaseConnection,
        key: &str,
        input: Map<String, Json>,
        locked: &[String],
    ) -> AdminResult<Record> {
        let model = self.find(db, key).await?;
        let mut am = model.clone().into_active_model();
        if !self.apply_input(&mut am, Some(&model), input, locked)? {
            return Ok(self.record(&model));
        }
        let updated = am.update(db).await?;
        Ok(self.record(&updated))
    }

    async fn delete(&self, db: &DatabaseConnection, keys: &[String]) -> AdminResult<u64> {
        let txn = db.begin().await?;
        for key in keys {
            let model = self.find(&txn, key).await?;
            model.into_active_model().delete(&txn).await?;
        }
        txn.commit().await?;
        Ok(keys.len() as u64)
    }

    async fn export(
        self: Arc<Self>,
        db: &DatabaseConnection,
        params: &ListParams,
        idle_limit: Duration,
    ) -> AdminResult<CsvStream> {
        let query = self.query(db.get_database_backend(), params)?;
        let header = csv_chunk([self.columns.iter().map(|c| c.name.as_str())])?;
        let export = Export {
            txn: Some(snapshot(db, idle_limit).await?),
            resource: self,
            query,
            after: None,
        };
        Ok(stream::once(async { Ok(header) })
            .chain(stream::try_unfold(export, Export::next))
            .boxed())
    }

    async fn lookup(
        &self,
        db: &DatabaseConnection,
        q: Option<&str>,
        column: Option<&str>,
        limit: u64,
    ) -> AdminResult<Vec<Choice>> {
        let value_col = match column {
            Some(name) => self.column(name)?,
            None => self
                .columns
                .iter()
                .find(|c| c.col.as_str() == self.pk[0].as_str())
                .ok_or(AdminError::NotFound)?,
        };
        let backend = db.get_database_backend();
        let sort = self.sort(&ListParams::default())?;
        let mut select = self.ordered(E::find(), &sort);
        if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
            // The search fields match `q`, or the picked value (usually the
            // key) starts with it or, if it is not text, equals it, so records
            // without text fields can be found too.
            let picked = if value_col.kind.is_textual() {
                Some(like(
                    backend,
                    Expr::col((E::default(), value_col.col)),
                    format!("{}%", escape_like(q)),
                ))
            } else {
                Self::exact(value_col, q).map(|v| value_col.col.eq(v))
            };
            select = select.filter(self.search_condition(backend, q).add_option(picked));
        }
        let value_col = value_col.col;
        let models = select.limit(limit).all(db).await?;
        Ok(models
            .iter()
            .map(|m| {
                let record = self.record(m);
                Choice {
                    key: record.key,
                    value: to_json(&m.get(value_col)),
                    label: record.label,
                }
            })
            .collect())
    }

    async fn labels(
        &self,
        db: &DatabaseConnection,
        column: &str,
        values: &[Json],
    ) -> AdminResult<BTreeMap<String, String>> {
        let column = self.column(column)?;
        let values: Vec<Value> = values
            .iter()
            .filter(|v| !v.is_null())
            .filter_map(|v| Self::convert(column, v.clone()).ok())
            .collect();
        if values.is_empty() {
            return Ok(BTreeMap::new());
        }
        let models = E::find().filter(column.col.is_in(values)).all(db).await?;
        Ok(models
            .iter()
            .map(|m| {
                let record = self.record(m);
                (json_to_text(&to_json(&m.get(column.col))), record.label)
            })
            .collect())
    }

    async fn run_action(
        &self,
        ctx: &AppContext,
        name: &str,
        keys: &[String],
    ) -> AdminResult<String> {
        let action = self
            .actions
            .iter()
            .find(|a| a.name == name)
            .ok_or(AdminError::NotFound)?;
        let mut models = Vec::with_capacity(keys.len());
        for key in keys {
            models.push(self.find(&ctx.db, key).await?);
        }
        (action.handler)(ctx.clone(), models)
            .await
            .map_err(|err| action_error(name, err))
    }
}

/// Cut `models` down to `per_page`, returning whether there were more.
fn truncate<M>(models: &mut Vec<M>, per_page: u64) -> bool {
    let per_page = usize::try_from(per_page).unwrap_or(usize::MAX);
    let more = models.len() > per_page;
    models.truncate(per_page);
    more
}

/// What the client learns of a failed action, following Loco's own rule for
/// controllers: a `BadRequest` message is meant for the user, other errors are
/// logged and reported as internal. Database errors read as they do for the
/// admin's own writes, so hook and validation messages still show.
fn action_error(action: &str, err: loco_rs::Error) -> AdminError {
    match err {
        loco_rs::Error::BadRequest(message) => AdminError::Validation {
            message,
            fields: FieldErrors::new(),
        },
        loco_rs::Error::WithBacktrace { inner, .. } => action_error(action, *inner),
        loco_rs::Error::DB(err) => AdminError::from(err),
        other => AdminError::Internal(format!("action `{action}` failed: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_is_a_guarded_read_only_snapshot() {
        let url = std::env::var("DATABASE_URL").unwrap_or_default();
        if !url.starts_with("postgres") {
            return;
        }
        let db = sea_orm::Database::connect(&url).await.unwrap();
        let txn = snapshot(&db, Duration::from_millis(1_500)).await.unwrap();
        let mut settings = Vec::new();
        for name in [
            "transaction_isolation",
            "transaction_read_only",
            "idle_in_transaction_session_timeout",
        ] {
            let sql = format!("SHOW {name}");
            let row = txn
                .query_one_raw(sea_orm::Statement::from_string(DbBackend::Postgres, sql))
                .await
                .unwrap()
                .unwrap();
            settings.push(row.try_get_by_index::<String>(0).unwrap());
        }
        assert_eq!(settings, ["repeatable read", "on", "1500ms"]);
    }

    #[test]
    fn like_patterns_are_escaped() {
        assert_eq!(escape_like("50%_off\\"), "50\\%\\_off\\\\");
        assert_eq!(escape_like("plain"), "plain");
    }

    #[test]
    fn csv_cells_neutralize_formulas() {
        assert_eq!(csv_cell(FieldKind::String, "=1+1".into()), "'=1+1");
        assert_eq!(csv_cell(FieldKind::Text, "@cmd".into()), "'@cmd");
        assert_eq!(csv_cell(FieldKind::Integer, "-5".into()), "-5");
        assert_eq!(csv_cell(FieldKind::String, "safe".into()), "safe");
    }

    #[test]
    fn blank_inputs_are_null_for_non_text_fields() {
        assert_eq!(normalize(FieldKind::Integer, Json::from(" ")), Json::Null);
        assert_eq!(normalize(FieldKind::String, Json::from("")), Json::from(""));
        assert_eq!(normalize(FieldKind::Integer, Json::from(4)), Json::from(4));
    }
}

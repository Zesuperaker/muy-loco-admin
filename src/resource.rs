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
    DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, EntityTrait, IdenStatic, Identity,
    IntoActiveModel, IsolationLevel, Iterable, ModelTrait, Order, PaginatorTrait,
    PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
    Select, TransactionTrait, Value,
    sea_query::{
        Alias, ColumnType, Expr, ExprTrait, Func, IntoIden, LikeExpr, TableName, TableRef,
        ValueTuple,
    },
};
use serde::Serialize;
use serde_json::{Map, Value as Json};

use crate::{
    error::{AdminError, AdminResult, FieldErrors},
    meta::{
        ActionMeta, Capabilities, FieldKind, FieldMeta, ModelMeta, Ordering, RelationMeta,
        SortDirection, enum_choices, humanize, max_length,
    },
    model::{Action, ModelAdmin},
    query::{ListParams, NOT_NULL_FILTER, NULL_FILTER},
    value::{convert, decode_key, encode_key, is_null, json_to_text, to_json, to_text},
};

const DEFAULT_PER_PAGE: u64 = 25;
const EXPORT_BATCH: u64 = 500;
const SENSITIVE: &[&str] = &["password", "secret", "token", "api_key", "salt"];
/// Loco's timestamp columns: read-only by default and listed after the other fields.
const TIMESTAMPS: &[&str] = &["created_at", "updated_at"];
const DISPLAY_CANDIDATES: &[&str] = &[
    "name", "title", "label", "email", "username", "slug", "code",
];

/// One record as sent to the UI.
#[derive(Debug, Clone, Serialize)]
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
#[allow(clippy::struct_field_names)]
pub struct Page {
    pub records: Vec<Record>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub pages: u64,
}

/// An option offered by a foreign-key picker.
#[derive(Debug, Clone, Serialize)]
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
    async fn count(&self, db: &DatabaseConnection) -> AdminResult<u64>;
    async fn list(&self, db: &DatabaseConnection, params: &ListParams) -> AdminResult<Page>;
    async fn get(&self, db: &DatabaseConnection, key: &str) -> AdminResult<Record>;
    async fn create(
        &self,
        db: &DatabaseConnection,
        input: Map<String, Json>,
    ) -> AdminResult<Record>;
    async fn update(
        &self,
        db: &DatabaseConnection,
        key: &str,
        input: Map<String, Json>,
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
fn is_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE.iter().any(|s| lower.contains(s))
}

/// Spellings of `q` to look for in `lower(column)`. SQLite's `lower()`, and
/// Postgres's under the `C` locale, only lowercase ASCII, so non-ASCII
/// letters keep their stored case: `q` is tried as typed and in lower, upper
/// and title case, with ASCII lowercased like `lower()` does.
fn search_terms(q: &str) -> Vec<String> {
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

/// `lower(column)` as text, for case-insensitive `LIKE`.
fn lowered<E: EntityTrait>(column: &Column<E>) -> Expr {
    let mut expr = Expr::col((E::default(), column.col));
    if !column.kind.is_textual() {
        expr = expr.cast_as(Alias::new("text"));
    }
    Expr::expr(Func::lower(expr))
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

    fn query(&self, params: &ListParams) -> AdminResult<Select<E>> {
        let mut cond = Condition::all();
        if let Some(q) = &params.q
            && !self.search.is_empty()
        {
            cond = cond.add(self.search_condition(q));
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

    /// Records whose search fields contain `q`, ignoring case.
    fn search_condition(&self, q: &str) -> Condition {
        let mut any = Condition::any();
        for term in search_terms(q) {
            let pattern = format!("%{}%", escape_like(&term));
            for idx in &self.search {
                any = any.add(
                    lowered(&self.columns[*idx]).like(LikeExpr::new(pattern.clone()).escape('\\')),
                );
            }
        }
        any
    }

    fn ordered(&self, params: &ListParams) -> AdminResult<Select<E>> {
        let (col, default_dir) = match &params.sort {
            Some(field) => (self.column(field)?.col, SortDirection::Asc),
            None => self.order,
        };
        let json = self
            .columns
            .iter()
            .any(|c| c.col.as_str() == col.as_str() && c.kind == FieldKind::Json);
        let order = match params.order.unwrap_or(default_dir) {
            SortDirection::Asc => Order::Asc,
            SortDirection::Desc => Order::Desc,
        };
        let query = self.query(params)?;
        // Postgres's `json` type has no ordering; its text form does.
        let mut query = if json {
            query.order_by(
                Expr::col((E::default(), col)).cast_as(Alias::new("text")),
                order.clone(),
            )
        } else {
            query.order_by(col, order.clone())
        };
        for pk in &self.pk {
            if pk.as_str() != col.as_str() {
                query = query.order_by(*pk, order.clone());
            }
        }
        Ok(query)
    }

    fn apply_input(
        &self,
        am: &mut E::ActiveModel,
        current: Option<&E::Model>,
        input: Map<String, Json>,
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
            if column.readonly || (current.is_some() && column.primary_key) {
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
            Err(AdminError::Validation {
                message: "some fields are invalid".to_string(),
                fields: errors,
            })
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

    async fn count(&self, db: &DatabaseConnection) -> AdminResult<u64> {
        Ok(E::find().count(db).await?)
    }

    async fn list(&self, db: &DatabaseConnection, params: &ListParams) -> AdminResult<Page> {
        let per_page = params.per_page.unwrap_or(self.meta.per_page);
        // Databases take a signed 64-bit offset.
        let offset = (params.page - 1)
            .checked_mul(per_page)
            .filter(|offset| i64::try_from(*offset).is_ok())
            .ok_or_else(|| AdminError::bad_request("`page` is too large"))?;
        let query = self.ordered(params)?;
        let total = self.query(params)?.count(db).await?;
        let models = query.offset(offset).limit(per_page).all(db).await?;
        Ok(Page {
            records: models.iter().map(|m| self.record(m)).collect(),
            total,
            page: params.page,
            per_page,
            pages: total.div_ceil(per_page),
        })
    }

    async fn get(&self, db: &DatabaseConnection, key: &str) -> AdminResult<Record> {
        Ok(self.record(&self.find(db, key).await?))
    }

    async fn create(
        &self,
        db: &DatabaseConnection,
        input: Map<String, Json>,
    ) -> AdminResult<Record> {
        let mut am = <E::ActiveModel as ActiveModelBehavior>::new();
        self.apply_input(&mut am, None, input)?;
        let model = am.insert(db).await?;
        Ok(self.record(&model))
    }

    async fn update(
        &self,
        db: &DatabaseConnection,
        key: &str,
        input: Map<String, Json>,
    ) -> AdminResult<Record> {
        let model = self.find(db, key).await?;
        let mut am = model.clone().into_active_model();
        if !self.apply_input(&mut am, Some(&model), input)? {
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
        let query = self.query(params)?;
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
        let params = ListParams {
            page: 1,
            ..ListParams::default()
        };
        let mut select = self.ordered(&params)?;
        if let Some(q) = q.map(str::trim).filter(|q| !q.is_empty()) {
            // The search fields contain `q`, or the picked value (usually
            // the key) starts with it, so records without text fields can be
            // found too.
            let prefix = format!("{}%", escape_like(&q.to_lowercase()));
            select = select.filter(
                self.search_condition(q)
                    .add(lowered(value_col).like(LikeExpr::new(prefix).escape('\\'))),
            );
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

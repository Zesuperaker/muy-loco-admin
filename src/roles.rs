//! Roles: what each admin user may do, created and edited in the admin UI.
//!
//! Roles are rows of the `admin_roles` table, which [`migration`] creates
//! along with a nullable role column on the users table. An admin user
//! without a role has full access; one with a role may only do what the role
//! grants, and never more than each model's [`ModelAdmin`](crate::ModelAdmin)
//! allows. Only users with full access can manage roles, assign them, or
//! change the other columns that decide who may use the admin.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, DbErr,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
    prelude::ChronoUtc,
    sea_query::{Alias, Expr, ExprTrait, Query},
};
use sea_orm_migration::{
    MigrationName, MigrationTrait, SchemaManager,
    prelude::{ForeignKey, ForeignKeyAction, Table},
    schema::{big_integer_null, big_pk_auto, string_uniq, text, timestamp_with_time_zone},
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::UserRoles,
    error::{AdminError, AdminResult, FieldErrors},
    meta::{Capabilities, ModelMeta},
};

/// The table roles are kept in.
pub(crate) const ROLES_TABLE: &str = "admin_roles";
/// The column [`migration`] adds to the users table.
pub(crate) const ROLE_COLUMN: &str = "admin_role_id";
/// The name the migration is recorded under. It must never change: apps that
/// ran it would fail to migrate.
const MIGRATION: &str = "m20261108_000001_muy_loco_admin_roles";
const MAX_NAME: usize = 100;

/// The migration that turns roles on, to add to the app's `Migrator` at the
/// end of its list, above `inject-above`:
///
/// ```rust,ignore
/// // migration/src/lib.rs
/// fn migrations() -> Vec<Box<dyn MigrationTrait>> {
///     vec![
///         Box::new(m20220101_000001_users::Migration),
///         muy_loco_admin::migrations::roles("users"),
///         // inject-above (do not remove this comment)
///     ]
/// }
/// ```
///
/// It creates the `admin_roles` table and adds a nullable `admin_role_id`
/// column to `users_table`, the table of the users who sign in to the admin.
/// A role cannot be deleted while users have it.
#[must_use]
pub fn migration(users_table: &str) -> Box<dyn MigrationTrait> {
    Box::new(CreateRoles {
        users: users_table.to_string(),
    })
}

struct CreateRoles {
    users: String,
}

impl MigrationName for CreateRoles {
    fn name(&self) -> &str {
        MIGRATION
    }
}

#[async_trait]
impl MigrationTrait for CreateRoles {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(ROLES_TABLE)
                .col(big_pk_auto("id"))
                .col(string_uniq("name"))
                .col(text("permissions"))
                .col(timestamp_with_time_zone("created_at").default(Expr::current_timestamp()))
                .col(timestamp_with_time_zone("updated_at").default(Expr::current_timestamp()))
                .to_owned(),
        )
        .await?;
        if m.get_database_backend() == DbBackend::Sqlite {
            // SQLite cannot add a foreign key to a table, but it can add a
            // column that references another table.
            let users = self.users.replace('"', "\"\"");
            m.get_connection()
                .execute_unprepared(&format!(
                    "ALTER TABLE \"{users}\" ADD COLUMN \"{ROLE_COLUMN}\" bigint NULL \
                     REFERENCES \"{ROLES_TABLE}\" (\"id\") ON DELETE RESTRICT"
                ))
                .await?;
            return Ok(());
        }
        m.alter_table(
            Table::alter()
                .table(Alias::new(&self.users))
                .add_column(big_integer_null(ROLE_COLUMN))
                .add_foreign_key(
                    ForeignKey::create()
                        .name(format!("fk-{}-{ROLE_COLUMN}-to-{ROLES_TABLE}", self.users))
                        .from_tbl(Alias::new(&self.users))
                        .from_col(ROLE_COLUMN)
                        .to_tbl(ROLES_TABLE)
                        .to_col("id")
                        .on_delete(ForeignKeyAction::Restrict)
                        .on_update(ForeignKeyAction::Cascade)
                        .get_foreign_key(),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.alter_table(
            Table::alter()
                .table(Alias::new(&self.users))
                .drop_column(ROLE_COLUMN)
                .to_owned(),
        )
        .await?;
        m.drop_table(Table::drop().table(ROLES_TABLE).to_owned())
            .await
    }
}

pub(crate) mod entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "admin_roles")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        #[sea_orm(unique)]
        pub name: String,
        /// [`Permissions`](super::Permissions) as JSON.
        #[sea_orm(column_type = "Text")]
        pub permissions: String,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// What a role may do with one model. Any other permission implies `view`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Grant {
    pub view: bool,
    pub create: bool,
    pub update: bool,
    pub delete: bool,
    pub export: bool,
    /// Names of the custom actions the role may run.
    pub actions: BTreeSet<String>,
}

impl Grant {
    fn grants_anything(&self) -> bool {
        self.view
            || self.create
            || self.update
            || self.delete
            || self.export
            || !self.actions.is_empty()
    }
}

/// Model name to what the role may do with it. Models not listed are hidden.
pub(crate) type Permissions = BTreeMap<String, Grant>;

/// A role as sent to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Role {
    pub id: i64,
    pub name: String,
    pub permissions: Permissions,
}

impl From<entity::Model> for Role {
    fn from(model: entity::Model) -> Self {
        // A role whose permissions cannot be read grants nothing rather than
        // everything, and can still be listed, edited and deleted.
        let permissions = serde_json::from_str(&model.permissions).unwrap_or_else(|e| {
            tracing::error!(role = model.id, error = %e, "role has unreadable permissions; it grants nothing");
            Permissions::new()
        });
        Self {
            id: model.id,
            name: model.name,
            permissions,
        }
    }
}

/// What the signed-in user may do.
#[derive(Debug, Clone)]
pub(crate) enum Access {
    /// An admin without a role: everything each model allows.
    Full,
    /// Only what the role grants.
    Role(Role),
}

impl Access {
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// What the user may do with a model: its capabilities, narrowed by the
    /// role.
    pub fn capabilities(&self, meta: &ModelMeta) -> Capabilities {
        let caps = meta.capabilities;
        let Self::Role(role) = self else {
            return caps;
        };
        let Some(grant) = role.permissions.get(&meta.name) else {
            return Capabilities::default();
        };
        Capabilities {
            view: caps.view && grant.grants_anything(),
            create: caps.create && grant.create,
            update: caps.update && grant.update,
            delete: caps.delete && grant.delete,
            export: caps.export && grant.export,
        }
    }

    pub fn can_view(&self, meta: &ModelMeta) -> bool {
        self.capabilities(meta).view
    }

    /// Whether the user may run the model's custom action `action`.
    pub fn can_run(&self, meta: &ModelMeta, action: &str) -> bool {
        match self {
            Self::Full => true,
            Self::Role(role) => role
                .permissions
                .get(&meta.name)
                .is_some_and(|grant| grant.actions.contains(action)),
        }
    }
}

/// A role as submitted by the UI. Fields left out are kept on update.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoleInput {
    pub name: Option<String>,
    pub permissions: Option<Permissions>,
}

fn invalid(field: &str, message: impl Into<String>) -> AdminError {
    AdminError::invalid(FieldErrors::from([(
        field.to_string(),
        vec![message.into()],
    )]))
}

/// Fit `permissions` to the registered models: models and actions that are
/// not registered (any more) and permissions a model does not offer are
/// dropped, any permission implies `view`, and models granted nothing are
/// left out. A role saved before a model or action was removed can then
/// still be saved.
pub(crate) fn normalize(models: &[&ModelMeta], permissions: Permissions) -> Permissions {
    let mut out = Permissions::new();
    for (name, grant) in permissions {
        let Some(meta) = models.iter().find(|m| m.name == name) else {
            continue;
        };
        let caps = meta.capabilities;
        let mut grant = Grant {
            view: grant.view,
            create: grant.create && caps.create,
            update: grant.update && caps.update,
            delete: grant.delete && caps.delete,
            export: grant.export && caps.export,
            actions: grant
                .actions
                .into_iter()
                .filter(|a| meta.actions.iter().any(|known| &known.name == a))
                .collect(),
        };
        if grant.grants_anything() {
            grant.view = true;
            out.insert(name, grant);
        }
    }
    out
}

fn valid_name(name: &str) -> AdminResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("name", "must not be empty"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(invalid(
            "name",
            format!("must be at most {MAX_NAME} characters"),
        ));
    }
    Ok(name.to_string())
}

async fn check_unique(db: &DatabaseConnection, name: &str, except: Option<i64>) -> AdminResult<()> {
    let mut query = entity::Entity::find().filter(entity::Column::Name.eq(name));
    if let Some(id) = except {
        query = query.filter(entity::Column::Id.ne(id));
    }
    if query.one(db).await?.is_some() {
        return Err(invalid("name", "is already taken"));
    }
    Ok(())
}

fn to_json(permissions: &Permissions) -> AdminResult<String> {
    serde_json::to_string(permissions).map_err(|e| AdminError::Internal(e.to_string()))
}

pub(crate) async fn find(db: &DatabaseConnection, id: i64) -> AdminResult<Option<Role>> {
    Ok(entity::Entity::find_by_id(id)
        .one(db)
        .await?
        .map(Role::from))
}

pub(crate) async fn all(db: &DatabaseConnection) -> AdminResult<Vec<Role>> {
    Ok(entity::Entity::find()
        .order_by_asc(entity::Column::Name)
        .order_by_asc(entity::Column::Id)
        .all(db)
        .await?
        .into_iter()
        .map(Role::from)
        .collect())
}

/// Names of the roles with these ids.
pub(crate) async fn names(
    db: &DatabaseConnection,
    ids: Vec<i64>,
) -> AdminResult<BTreeMap<i64, String>> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(entity::Entity::find()
        .filter(entity::Column::Id.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|role| (role.id, role.name))
        .collect())
}

pub(crate) async fn create(
    db: &DatabaseConnection,
    models: &[&ModelMeta],
    input: RoleInput,
) -> AdminResult<Role> {
    let name = valid_name(input.name.as_deref().unwrap_or_default())?;
    let permissions = normalize(models, input.permissions.unwrap_or_default());
    check_unique(db, &name, None).await?;
    let now = ChronoUtc::now().fixed_offset();
    let model = entity::ActiveModel {
        name: Set(name),
        permissions: Set(to_json(&permissions)?),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(Role::from(model))
}

pub(crate) async fn update(
    db: &DatabaseConnection,
    models: &[&ModelMeta],
    id: i64,
    input: RoleInput,
) -> AdminResult<Role> {
    let current = entity::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(AdminError::NotFound)?;
    let mut am = current.into_active_model();
    if let Some(name) = input.name {
        let name = valid_name(&name)?;
        check_unique(db, &name, Some(id)).await?;
        am.name = Set(name);
    }
    if let Some(permissions) = input.permissions {
        am.permissions = Set(to_json(&normalize(models, permissions))?);
    }
    am.updated_at = Set(ChronoUtc::now().fixed_offset());
    Ok(Role::from(am.update(db).await?))
}

/// Delete a role, unless users still have it.
pub(crate) async fn delete(db: &DatabaseConnection, users: &UserRoles, id: i64) -> AdminResult<()> {
    let assigned = assigned(db, users, Some(id))
        .await?
        .get(&id)
        .copied()
        .unwrap_or(0);
    if assigned > 0 {
        let who = if assigned == 1 {
            "1 user"
        } else {
            &format!("{assigned} users")
        };
        return Err(AdminError::Conflict(format!(
            "this role is assigned to {who}; give them another role first"
        )));
    }
    let deleted = entity::Entity::delete_by_id(id).exec(db).await?;
    if deleted.rows_affected == 0 {
        return Err(AdminError::NotFound);
    }
    Ok(())
}

/// How many users have each role, or only role `only`.
pub(crate) async fn assigned(
    db: &DatabaseConnection,
    users: &UserRoles,
    only: Option<i64>,
) -> AdminResult<BTreeMap<i64, u64>> {
    // Qualified, since SQLite reads an unknown quoted name as a string.
    let column = (Alias::new(&users.table), Alias::new(&users.column));
    let mut query = Query::select();
    query
        .column(column.clone())
        .expr_as(Expr::col(column.clone()).count(), "users")
        .from(Alias::new(&users.table))
        .and_where(Expr::col(column.clone()).is_not_null())
        .group_by_col(column.clone());
    if let Some(id) = only {
        query.and_where(Expr::col(column).eq(id));
    }
    let mut counts = BTreeMap::new();
    for row in db.query_all(&query).await? {
        let id: i64 = row.try_get("", &users.column)?;
        let count: i64 = row.try_get("", "users")?;
        counts.insert(id, u64::try_from(count).unwrap_or_default());
    }
    Ok(counts)
}

/// Why roles cannot work with this database, if they cannot: the roles
/// table or the users' role column is missing.
pub(crate) async fn check_tables(db: &DatabaseConnection, users: &UserRoles) -> Result<(), String> {
    // Qualified, since SQLite reads an unknown quoted name as a string.
    let roles = Query::select()
        .column((ROLES_TABLE, "id"))
        .from(ROLES_TABLE)
        .limit(1)
        .to_owned();
    let user = Query::select()
        .column((Alias::new(&users.table), Alias::new(&users.column)))
        .from(Alias::new(&users.table))
        .limit(1)
        .to_owned();
    for query in [roles, user] {
        if let Err(err) = db.query_one(&query).await {
            return Err(format!(
                "roles need the `{ROLES_TABLE}` table and the `{}.{}` column: add \
                 `muy_loco_admin::migrations::roles(\"{}\")` to the app's `Migrator` and \
                 run `cargo loco db migrate` ({err})",
                users.table, users.column, users.table
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ActionMeta, Ordering, SortDirection};

    fn meta(name: &str, caps: Capabilities, actions: &[&str]) -> ModelMeta {
        ModelMeta {
            name: name.to_string(),
            label: name.to_string(),
            table: name.to_string(),
            primary_key: vec!["id".to_string()],
            fields: Vec::new(),
            list_display: Vec::new(),
            search_fields: Vec::new(),
            list_filter: Vec::new(),
            ordering: Ordering {
                field: "id".to_string(),
                direction: SortDirection::Desc,
            },
            per_page: 25,
            display_field: None,
            actions: actions
                .iter()
                .map(|a| ActionMeta {
                    name: (*a).to_string(),
                    label: (*a).to_string(),
                })
                .collect(),
            related: Vec::new(),
            capabilities: caps,
        }
    }

    const ALL: Capabilities = Capabilities {
        view: true,
        create: true,
        update: true,
        delete: true,
        export: true,
    };

    fn grant(json: serde_json::Value) -> Grant {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn permissions_are_normalized() {
        let posts = meta("posts", ALL, &["publish"]);
        let tags = meta(
            "tags",
            Capabilities {
                create: false,
                ..ALL
            },
            &[],
        );
        let models = [&posts, &tags];
        let input = Permissions::from([
            (
                "posts".to_string(),
                grant(serde_json::json!({"update": true, "actions": ["publish"]})),
            ),
            (
                "tags".to_string(),
                grant(serde_json::json!({"create": true})),
            ),
        ]);
        let out = normalize(&models, input);
        assert_eq!(
            out,
            Permissions::from([(
                "posts".to_string(),
                grant(serde_json::json!({"view": true, "update": true, "actions": ["publish"]}))
            )])
        );

        // Models and actions that are no longer registered are dropped.
        let stale = Permissions::from([
            ("gone".to_string(), grant(serde_json::json!({"view": true}))),
            (
                "posts".to_string(),
                grant(serde_json::json!({"view": true, "actions": ["publish", "archive"]})),
            ),
            (
                "tags".to_string(),
                grant(serde_json::json!({"actions": ["publish"]})),
            ),
        ]);
        assert_eq!(
            normalize(&models, stale),
            Permissions::from([(
                "posts".to_string(),
                grant(serde_json::json!({"view": true, "actions": ["publish"]}))
            )])
        );
        assert!(serde_json::from_value::<Grant>(serde_json::json!({"drop": true})).is_err());
    }

    #[test]
    fn roles_narrow_model_capabilities() {
        let posts = meta(
            "posts",
            Capabilities {
                delete: false,
                ..ALL
            },
            &["publish", "archive"],
        );
        let users = meta("users", ALL, &[]);
        let role = Access::Role(Role {
            id: 1,
            name: "Editor".to_string(),
            permissions: Permissions::from([(
                "posts".to_string(),
                grant(serde_json::json!({"delete": true, "update": true, "actions": ["publish"]})),
            )]),
        });
        let caps = role.capabilities(&posts);
        assert!(caps.view && caps.update);
        assert!(!caps.create && !caps.delete && !caps.export);
        assert!(role.can_run(&posts, "publish"));
        assert!(!role.can_run(&posts, "archive"));
        assert!(!role.can_view(&users));
        assert!(!role.can_run(&users, "publish"));
        assert!(!role.is_full());

        assert!(Access::Full.is_full());
        assert!(Access::Full.can_view(&users));
        assert!(Access::Full.can_run(&posts, "archive"));
        assert!(!Access::Full.capabilities(&posts).delete);
    }

    #[test]
    fn names_are_validated() {
        assert_eq!(valid_name("  Editor ").unwrap(), "Editor");
        assert!(valid_name(" ").is_err());
        assert!(valid_name(&"x".repeat(MAX_NAME)).is_ok());
        assert!(valid_name(&"x".repeat(MAX_NAME + 1)).is_err());
    }

    #[test]
    fn unreadable_permissions_grant_nothing() {
        let now = ChronoUtc::now().fixed_offset();
        let model = entity::Model {
            id: 3,
            name: "Broken".to_string(),
            permissions: "not json".to_string(),
            created_at: now,
            updated_at: now,
        };
        let role = Role::from(model);
        assert_eq!(role.name, "Broken");
        assert!(role.permissions.is_empty());
    }
}

//! Per-model admin configuration.

use std::{future::Future, marker::PhantomData, pin::Pin, sync::Arc};

use loco_rs::app::AppContext;
use sea_orm::EntityTrait;

use crate::meta::SortDirection;

/// What a custom action returns: a message shown to the user on success.
pub type ActionResult = loco_rs::Result<String>;

type ActionFuture = Pin<Box<dyn Future<Output = ActionResult> + Send>>;
type ActionHandler<M> = Arc<dyn Fn(AppContext, Vec<M>) -> ActionFuture + Send + Sync>;

pub(crate) struct Action<E: EntityTrait> {
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) handler: ActionHandler<E::Model>,
}

/// Configuration of one entity in the admin.
///
/// Field names are column names (`users::Column::Email.as_str()`). Unknown
/// names are reported when the admin is mounted.
///
/// ```rust,ignore
/// ModelAdmin::<posts::Entity>::new()
///     .list_display(["id", "title", "published", "user_id"])
///     .search_fields(["title", "body"])
///     .list_filter(["published", "user_id"])
///     .action("publish", "Publish", |ctx, posts| async move {
///         // ...
///         Ok(format!("published {} posts", posts.len()))
///     })
/// ```
#[allow(clippy::struct_excessive_bools)]
pub struct ModelAdmin<E: EntityTrait> {
    pub(crate) name: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) list_display: Option<Vec<String>>,
    pub(crate) search_fields: Option<Vec<String>>,
    pub(crate) list_filter: Option<Vec<String>>,
    pub(crate) readonly_fields: Option<Vec<String>>,
    pub(crate) hidden_fields: Option<Vec<String>>,
    pub(crate) ordering: Option<(String, SortDirection)>,
    pub(crate) per_page: Option<u64>,
    pub(crate) display_field: Option<String>,
    pub(crate) can_create: bool,
    pub(crate) can_update: bool,
    pub(crate) can_delete: bool,
    pub(crate) can_export: bool,
    pub(crate) actions: Vec<Action<E>>,
    _entity: PhantomData<E>,
}

impl<E: EntityTrait> Default for ModelAdmin<E> {
    fn default() -> Self {
        Self::new()
    }
}

fn names(items: impl IntoIterator<Item = impl Into<String>>) -> Vec<String> {
    items.into_iter().map(Into::into).collect()
}

impl<E: EntityTrait> ModelAdmin<E> {
    /// Default configuration: every column visible except sensitive ones,
    /// all operations enabled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: None,
            label: None,
            list_display: None,
            search_fields: None,
            list_filter: None,
            readonly_fields: None,
            hidden_fields: None,
            ordering: None,
            per_page: None,
            display_field: None,
            can_create: true,
            can_update: true,
            can_delete: true,
            can_export: true,
            actions: Vec::new(),
            _entity: PhantomData,
        }
    }

    /// URL name of the model (defaults to the table name).
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Display name (defaults to the humanized table name).
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Columns shown in the list (defaults to all but text, JSON and binary
    /// columns).
    #[must_use]
    pub fn list_display(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.list_display = Some(names(fields));
        self
    }

    /// Columns matched by the search box (defaults to text columns).
    #[must_use]
    pub fn search_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.search_fields = Some(names(fields));
        self
    }

    /// Columns offered as list filters (defaults to boolean, enum and
    /// foreign-key columns).
    #[must_use]
    pub fn list_filter(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.list_filter = Some(names(fields));
        self
    }

    /// Columns that are shown but cannot be edited (defaults to `created_at`
    /// and `updated_at`). Auto-increment keys are always read-only.
    #[must_use]
    pub fn readonly_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.readonly_fields = Some(names(fields));
        self
    }

    /// Columns never sent to or accepted from the UI. Defaults to columns
    /// whose name contains `password`, `secret`, `token`, `api_key` or
    /// `salt`. Setting this replaces the default list.
    #[must_use]
    pub fn hidden_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.hidden_fields = Some(names(fields));
        self
    }

    /// Default list ordering (defaults to the primary key, descending).
    #[must_use]
    pub fn ordering(mut self, field: impl Into<String>, direction: SortDirection) -> Self {
        self.ordering = Some((field.into(), direction));
        self
    }

    /// Default page size (defaults to 25, at most 500).
    #[must_use]
    pub fn per_page(mut self, per_page: u64) -> Self {
        self.per_page = Some(per_page);
        self
    }

    /// Column used to label records in links and pickers (defaults to the
    /// first of `name`, `title`, `label`, `email`, `username`, `slug`,
    /// `code`).
    #[must_use]
    pub fn display_field(mut self, field: impl Into<String>) -> Self {
        self.display_field = Some(field.into());
        self
    }

    /// Allow creating records.
    #[must_use]
    pub fn can_create(mut self, allowed: bool) -> Self {
        self.can_create = allowed;
        self
    }

    /// Allow editing records.
    #[must_use]
    pub fn can_update(mut self, allowed: bool) -> Self {
        self.can_update = allowed;
        self
    }

    /// Allow deleting records.
    #[must_use]
    pub fn can_delete(mut self, allowed: bool) -> Self {
        self.can_delete = allowed;
        self
    }

    /// Allow CSV export.
    #[must_use]
    pub fn can_export(mut self, allowed: bool) -> Self {
        self.can_export = allowed;
        self
    }

    /// Disable create, update and delete.
    #[must_use]
    pub fn read_only(self) -> Self {
        self.can_create(false).can_update(false).can_delete(false)
    }

    /// Add a bulk action run on the selected records. The returned message
    /// is shown to the user; an error is shown as a failure.
    #[must_use]
    pub fn action<F, Fut>(
        mut self,
        name: impl Into<String>,
        label: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(AppContext, Vec<E::Model>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ActionResult> + Send + 'static,
    {
        self.actions.push(Action {
            name: name.into(),
            label: label.into(),
            handler: Arc::new(move |ctx, models| Box::pin(handler(ctx, models))),
        });
        self
    }
}

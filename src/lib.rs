//! An auto-generated admin dashboard for [Loco](https://loco.rs) apps.
//!
//! The admin reads `SeaORM` entity metadata at runtime and serves an embedded
//! React UI. Writes go through `ActiveModel`, so model hooks and validation
//! run as they do in app code.
//!
//! ```rust,ignore
//! use muy_loco_admin::{Admin, ModelAdmin, auth::JwtAuth};
//!
//! async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
//!     Ok(vec![Box::new(
//!         Admin::new(JwtAuth::<users::Model>::new(|user| user.is_admin))
//!             .register(ModelAdmin::<users::Entity>::new().can_create(false))
//!             .model::<posts::Entity>()
//!             .middlewares_from::<Self>(),
//!     )])
//! }
//! ```

mod admin;
mod api;
mod assets;
pub mod auth;
mod error;
mod meta;
mod model;
mod query;
mod resource;
mod roles;
mod session;
mod task;
mod value;

pub use admin::{Admin, CONFIG_KEY};
pub use api::{CSRF_HEADER, CSRF_VALUE};
pub use auth::{AdminAuth, AdminUser, AllowAll, AuthError, FormLogin, LoginMethod, UserRoles};
pub use error::{AdminError, FieldErrors};
pub use meta::{FieldKind, SortDirection};
pub use model::{ActionResult, ModelAdmin};
pub use query::{MAX_PER_PAGE, NOT_NULL_FILTER, NULL_FILTER};
pub use task::PromoteAdmin;

/// The admin's migrations, to add to the app's `Migrator`.
///
/// Add each at the end of the list, above `inject-above`, so the list stays
/// in the order the migrations ran and `cargo loco db down` rolls back the
/// newest first. Later versions may add migrations here; add them the same
/// way.
pub mod migrations {
    pub use crate::roles::migration as roles;
}

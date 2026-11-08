#![allow(elided_lifetimes_in_paths)]
#![allow(clippy::wildcard_imports)]
pub use sea_orm_migration::prelude::*;
mod m20220101_000001_users;

mod m20261105_005058_add_is_admin_to_users;
mod m20261105_005106_categories;
mod m20261105_005124_articles;
mod m20261105_010000_tags;
mod m20261105_010100_article_tags;
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_users::Migration),
            Box::new(m20261105_005058_add_is_admin_to_users::Migration),
            Box::new(m20261105_005106_categories::Migration),
            Box::new(m20261105_005124_articles::Migration),
            Box::new(m20261105_010000_tags::Migration),
            Box::new(m20261105_010100_article_tags::Migration),
            muy_loco_admin::migrations::roles("users"),
            // inject-above (do not remove this comment)
        ]
    }
}

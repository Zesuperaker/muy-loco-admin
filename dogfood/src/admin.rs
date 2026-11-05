//! The admin dashboard served at `/admin`.

use loco_rs::app::AppContext;
use muy_loco_admin::{auth::JwtAuth, Admin, ModelAdmin, SortDirection};
use sea_orm::{ActiveModelTrait, ActiveValue, IntoActiveModel};

use crate::models::{
    _entities::{article_tags, articles, categories, tags, users},
    articles::ArticleStatus,
};

#[must_use]
pub fn admin() -> Admin {
    Admin::new(
        JwtAuth::<users::Model>::new(|user| user.is_admin).display_name(|user| user.email.clone()),
    )
    .title("Dogfood Admin")
    .register(
        ModelAdmin::<users::Entity>::new()
            .can_create(false)
            .display_field("email")
            .list_display(["id", "email", "name", "is_admin", "email_verified_at", "created_at"])
            .list_filter(["is_admin"])
            .readonly_fields([
                "pid",
                "created_at",
                "updated_at",
                "reset_sent_at",
                "email_verification_sent_at",
                "email_verified_at",
                "magic_link_expiration",
            ]),
    )
    .register(
        ModelAdmin::<articles::Entity>::new()
            .list_display([
                "id",
                "title",
                "status",
                "featured",
                "views",
                "price",
                "user_id",
                "category_id",
                "published_at",
            ])
            .action("publish", "Publish", |ctx: AppContext, articles| async move {
                let count = articles.len();
                for article in articles {
                    let mut am: articles::ActiveModel = article.into_active_model();
                    am.status = ActiveValue::Set(ArticleStatus::Published);
                    am.published_at = ActiveValue::Set(Some(chrono::Utc::now().into()));
                    am.update(&ctx.db).await?;
                }
                Ok(format!("Published {count} articles"))
            }),
    )
    .model::<categories::Entity>()
    .register(ModelAdmin::<tags::Entity>::new().ordering("name", SortDirection::Asc))
    .register(ModelAdmin::<article_tags::Entity>::new().label("Article tags"))
}

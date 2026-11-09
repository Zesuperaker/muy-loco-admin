//! The admin dashboard served at `/admin`.
//!
//! `models` is laid out like the file `cargo loco task admin:models` writes:
//! the task adds a line above the marker comment for each entity this file
//! does not mention yet.

use loco_rs::app::AppContext;
use muy_loco_admin::{auth::JwtAuth, Admin, ModelAdmin, SortDirection};
use sea_orm::{ActiveModelTrait, ActiveValue, IntoActiveModel};

use crate::models::{
    _entities::{self, users},
    articles::ArticleStatus,
};

#[must_use]
pub fn admin() -> Admin {
    models(Admin::new(
        JwtAuth::<users::Model>::new(|user| user.is_admin)
            .display_name(|user| user.email.clone())
            .roles(users::Column::AdminRoleId, [users::Column::IsAdmin]),
    ))
}

/// Registers the app's models on `admin`.
#[must_use]
pub fn models(admin: Admin) -> Admin {
    admin
        .register(
            ModelAdmin::<_entities::users::Entity>::new()
                .can_create(false)
                .display_field("email")
                .list_display([
                    "id",
                    "email",
                    "name",
                    "is_admin",
                    "admin_role_id",
                    "email_verified_at",
                    "created_at",
                ])
                .list_filter(["is_admin", "admin_role_id"])
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
            ModelAdmin::<_entities::articles::Entity>::new()
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
                .action(
                    "publish",
                    "Publish",
                    |ctx: AppContext, articles| async move {
                        let count = articles.len();
                        for article in articles {
                            let mut am: _entities::articles::ActiveModel =
                                article.into_active_model();
                            am.status = ActiveValue::Set(ArticleStatus::Published);
                            am.published_at = ActiveValue::Set(Some(chrono::Utc::now().into()));
                            am.update(&ctx.db).await?;
                        }
                        Ok(format!("Published {count} articles"))
                    },
                ),
        )
        .model::<_entities::categories::Entity>()
        .register(ModelAdmin::<_entities::tags::Entity>::new().ordering("name", SortDirection::Asc))
        .register(ModelAdmin::<_entities::article_tags::Entity>::new().label("Article tags"))
    // admin-models-inject (do not remove this comment)
}

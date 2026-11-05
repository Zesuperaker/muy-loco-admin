use loco_rs::schema::*;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        create_table(
            m,
            "articles",
            &[
                ("id", ColType::PkAuto),
                ("title", ColType::String),
                ("body", ColType::Text),
                (
                    "status",
                    ColType::EnumWithDefault(
                        "article_status".to_string(),
                        vec![
                            "draft".to_string(),
                            "review".to_string(),
                            "published".to_string(),
                        ],
                        "draft".to_string(),
                    ),
                ),
                ("featured", ColType::BooleanWithDefault(false)),
                ("views", ColType::BigIntegerWithDefault(0)),
                ("rating", ColType::DoubleNull),
                ("published_at", ColType::TimestampWithTimeZoneNull),
                ("price", ColType::DecimalLen(10, 2)),
                ("metadata", ColType::JsonBinaryNull),
            ],
            &[("user", ""), ("category?", "")],
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        drop_table(m, "articles").await
    }
}

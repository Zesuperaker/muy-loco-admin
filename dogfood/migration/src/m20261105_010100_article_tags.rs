use sea_orm_migration::prelude::*;

/// A join table with a composite primary key.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum ArticleTags {
    Table,
    ArticleId,
    TagId,
    Position,
}

#[derive(DeriveIden)]
enum Articles {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Tags {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.create_table(
            Table::create()
                .table(ArticleTags::Table)
                .col(
                    ColumnDef::new(ArticleTags::ArticleId)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(ArticleTags::TagId).big_integer().not_null())
                .col(
                    ColumnDef::new(ArticleTags::Position)
                        .small_integer()
                        .not_null()
                        .default(0),
                )
                .primary_key(
                    Index::create()
                        .col(ArticleTags::ArticleId)
                        .col(ArticleTags::TagId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk-article_tags-article_id")
                        .from(ArticleTags::Table, ArticleTags::ArticleId)
                        .to(Articles::Table, Articles::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk-article_tags-tag_id")
                        .from(ArticleTags::Table, ArticleTags::TagId)
                        .to(Tags::Table, Tags::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        m.drop_table(Table::drop().table(ArticleTags::Table).to_owned())
            .await
    }
}

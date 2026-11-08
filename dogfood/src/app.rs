use loco_rs::{
    app::{AppContext, Hooks, Initializer},
    bgworker::{BackgroundWorker, Queue},
    boot::{create_app, BootResult, StartMode},
    config::Config,
    controller::AppRoutes,
    db::{self, truncate_table},
    environment::Environment,
    prelude::async_trait,
    task::Tasks,
    Result,
};
use migration::Migrator;
use std::path::Path;

#[allow(unused_imports)]
use crate::{
    admin, controllers, initializers,
    models::_entities::{article_tags, articles, categories, tags, users},
    tasks,
    workers::downloader::DownloadWorker,
};

pub struct App;
#[async_trait]
impl Hooks for App {
    fn app_name() -> &'static str {
        env!("CARGO_CRATE_NAME")
    }

    fn app_version() -> String {
        format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            option_env!("BUILD_SHA")
                .or(option_env!("GITHUB_SHA"))
                .unwrap_or("dev")
        )
    }

    async fn boot(
        mode: StartMode,
        environment: &Environment,
        config: Config,
    ) -> Result<BootResult> {
        create_app::<Self, Migrator>(mode, environment, config).await
    }

    async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
        Ok(vec![
            Box::new(initializers::view_engine::ViewEngineInitializer),
            Box::new(admin::admin().middlewares_from::<Self>()),
        ])
    }

    fn routes(_ctx: &AppContext) -> AppRoutes {
        AppRoutes::with_default_routes() // controller routes below
            .add_route(controllers::auth::routes())
            .add_route(controllers::page::routes())
    }
    async fn connect_workers(ctx: &AppContext, queue: &Queue) -> Result<()> {
        queue.register(DownloadWorker::build(ctx)).await?;
        Ok(())
    }

    #[allow(unused_variables)]
    fn register_tasks(tasks: &mut Tasks) {
        // tasks-inject (do not remove)
        tasks.register(tasks::user_create::UserCreate);
        tasks.register(tasks::user_delete::UserDelete);
        tasks.register(muy_loco_admin::PromoteAdmin::new(
            users::Column::Email,
            users::Column::IsAdmin,
        ));
    }
    async fn truncate(ctx: &AppContext) -> Result<()> {
        truncate_table(&ctx.db, article_tags::Entity).await?;
        truncate_table(&ctx.db, articles::Entity).await?;
        truncate_table(&ctx.db, tags::Entity).await?;
        truncate_table(&ctx.db, categories::Entity).await?;
        truncate_table(&ctx.db, users::Entity).await?;
        Ok(())
    }
    async fn seed(ctx: &AppContext, base: &Path) -> Result<()> {
        let path = |name: &str| base.join(name).display().to_string();
        db::seed::<users::ActiveModel>(&ctx.db, &path("users.yaml")).await?;
        db::seed::<categories::ActiveModel>(&ctx.db, &path("categories.yaml")).await?;
        db::seed::<tags::ActiveModel>(&ctx.db, &path("tags.yaml")).await?;
        db::seed::<articles::ActiveModel>(&ctx.db, &path("articles.yaml")).await?;
        db::seed::<article_tags::ActiveModel>(&ctx.db, &path("article_tags.yaml")).await?;
        Ok(())
    }
}

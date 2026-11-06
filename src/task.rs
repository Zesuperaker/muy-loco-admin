//! A Loco task that gives a user admin access.

use async_trait::async_trait;
use loco_rs::{
    Error, Result,
    app::AppContext,
    task::{Task, TaskInfo, Vars},
};
use sea_orm::{ColumnTrait, EntityName, EntityTrait, QueryFilter, sea_query::Expr};

/// `cargo loco task admin:promote email:you@example.com` sets the boolean
/// `flag` column to true for the user whose `identifier` column (a text column
/// such as `email`) matches, so the first admin needs no SQL. The argument is
/// named after `identifier`.
///
/// ```rust,ignore
/// // src/app.rs, inside `impl Hooks for App`
/// fn register_tasks(tasks: &mut Tasks) {
///     tasks.register(PromoteAdmin::new(users::Column::Email, users::Column::IsAdmin));
/// }
/// ```
pub struct PromoteAdmin<C> {
    identifier: C,
    flag: C,
}

impl<C: ColumnTrait> PromoteAdmin<C> {
    /// Find users by `identifier` and set `flag` to true.
    #[must_use]
    pub fn new(identifier: C, flag: C) -> Self {
        Self { identifier, flag }
    }
}

#[async_trait]
impl<C> Task for PromoteAdmin<C>
where
    C: ColumnTrait + Send + Sync + 'static,
    C::EntityName: EntityTrait,
{
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "admin:promote".to_string(),
            detail: format!(
                "Give a user admin access by setting `{}` to true.\nUsage:\ncargo loco task admin:promote {}:you@example.com",
                self.flag.as_str(),
                self.identifier.as_str(),
            ),
        }
    }

    async fn run(&self, ctx: &AppContext, vars: &Vars) -> Result<()> {
        let key = self.identifier.as_str();
        let value = vars.cli_arg(key).map_err(|_| {
            Error::string(&format!(
                "missing `{key}`: cargo loco task admin:promote {key}:..."
            ))
        })?;
        let updated = <C::EntityName as EntityTrait>::update_many()
            .col_expr(self.flag, Expr::value(true))
            .filter(self.identifier.eq(value))
            .exec(&ctx.db)
            .await?;
        let table = C::EntityName::default().table_name().to_string();
        if updated.rows_affected == 0 {
            return Err(Error::string(&format!(
                "no row in `{table}` has {key} = {value}"
            )));
        }
        println!("{value} is now an admin");
        Ok(())
    }
}

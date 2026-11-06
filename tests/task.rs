mod common;

use common::{create_user, setup, users};
use loco_rs::task::{Tasks, Vars};
use muy_loco_admin::PromoteAdmin;
use sea_orm::EntityTrait;

fn email(value: &str) -> Vars {
    Vars::from_cli_args(vec![("email".to_string(), value.to_string())])
}

#[tokio::test]
async fn promote_admin_sets_the_flag() {
    let (db, ctx) = setup().await;
    let mut tasks = Tasks::default();
    tasks.register(PromoteAdmin::new(
        users::Column::Email,
        users::Column::IsAdmin,
    ));
    let info = &tasks.list()[0];
    assert_eq!(info.name, "admin:promote");
    assert!(
        info.detail.contains("admin:promote email:"),
        "{}",
        info.detail
    );

    let joe = create_user(&db.db, "joe@example.com", false).await;
    let ann = create_user(&db.db, "ann@example.com", false).await;
    let is_admin = |id| {
        let db = db.db.clone();
        async move {
            users::Entity::find_by_id(id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .is_admin
        }
    };

    tasks
        .run(&ctx, "admin:promote", &email("joe@example.com"))
        .await
        .unwrap();
    assert!(is_admin(joe.id).await);
    assert!(!is_admin(ann.id).await, "only the matching user");
    tasks
        .run(&ctx, "admin:promote", &email("joe@example.com"))
        .await
        .expect("promoting an admin again succeeds");

    let missing = tasks
        .run(&ctx, "admin:promote", &Vars::default())
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("missing `email`"), "{missing}");
    let unknown = tasks
        .run(&ctx, "admin:promote", &email("nobody@example.com"))
        .await
        .unwrap_err();
    assert!(
        unknown
            .to_string()
            .contains("no row in `users` has email = nobody@example.com"),
        "{unknown}"
    );
    assert!(!is_admin(ann.id).await);
}

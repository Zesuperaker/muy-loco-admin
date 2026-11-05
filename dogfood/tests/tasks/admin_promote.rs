use dogfood::{app::App, models::users};
use loco_rs::{boot::run_task, task, testing::prelude::*};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn can_promote_a_user_to_admin() {
    let boot = boot_test::<App>().await.unwrap();
    let db = &boot.app_context.db;
    let user = users::Model::create_with_password(
        db,
        &users::RegisterParams {
            email: "promote@example.com".to_string(),
            password: "securepassword".to_string(),
            name: "Promoted User".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(!user.is_admin);

    let vars = task::Vars::from_cli_args(vec![("email".to_string(), user.email.clone())]);
    run_task::<App>(&boot.app_context, Some(&"admin:promote".to_string()), &vars)
        .await
        .unwrap();

    let user = users::Model::find_by_email(db, "promote@example.com")
        .await
        .unwrap();
    assert!(user.is_admin);
}

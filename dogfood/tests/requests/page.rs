use dogfood::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

/// The server-side rendered root page links to the admin.
#[tokio::test]
#[serial]
async fn can_get_home_page() {
    request::<App, _, _>(|request, _ctx| async move {
        let res = request.get("/").await;

        assert_eq!(res.status_code(), 200);
        assert!(
            res.text().contains("Open the admin"),
            "expected the rendered view, got: {}",
            res.text()
        );
    })
    .await;
}

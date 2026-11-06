use loco_rs::controller::views::{engines, ViewRenderer};

#[test]
fn renders_home_view() {
    let view = engines::TeraView::build().expect("view engine should build");
    let rendered = view
        .render(
            "home/hello.html",
            serde_json::json!({"app_name": "Dogfood"}),
        )
        .expect("home view should render");
    assert!(rendered.contains("Open the admin"));
}

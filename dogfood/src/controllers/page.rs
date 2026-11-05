use loco_rs::prelude::*;

/// Serves the server-side rendered home page at `/`.
#[debug_handler]
async fn index(ViewEngine(v): ViewEngine<TeraView>) -> Result<Response> {
    format::render().view(&v, "home/hello.html", data!({"app_name": "Dogfood"}))
}

pub fn routes() -> Routes {
    Routes::new().add("/", get(index))
}

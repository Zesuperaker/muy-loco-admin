mod common;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use common::{create_post, create_user, is_postgres, post_tags, posts, setup, users};
use http_body_util::BodyExt;
use muy_loco_admin::{Admin, AllowAll};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    prelude::{ChronoUtc, Decimal},
};
use tower::ServiceExt;

fn router(ctx: &loco_rs::app::AppContext) -> Router {
    Admin::new(AllowAll)
        .model::<users::Entity>()
        .model::<posts::Entity>()
        .model::<post_tags::Entity>()
        .router(ctx)
        .unwrap()
}

async fn insert_posts(db: &DatabaseConnection, user_id: i32, count: usize, prefix: &str) {
    let now = ChronoUtc::now().fixed_offset();
    let rows: Vec<posts::ActiveModel> = (0..count)
        .map(|i| posts::ActiveModel {
            title: Set(format!("{prefix} {i}")),
            body: Set(String::new()),
            status: Set(posts::PostStatus::Draft),
            featured: Set(false),
            rating: Set(None),
            price: Set(Decimal::new(0, 0)),
            views: Set(0),
            published_on: Set(None),
            meta: Set(None),
            user_id: Set(user_id),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .collect();
    for chunk in rows.chunks(400) {
        posts::Entity::insert_many(chunk.to_vec())
            .exec(db)
            .await
            .expect("insert posts");
    }
}

/// Request an export and return the body as it arrives, one chunk per frame.
async fn export_body(router: &Router, uri: &str) -> Body {
    let res = router
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    res.into_body()
}

async fn next_chunk(body: &mut Body) -> Option<String> {
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.expect("frame").into_data() {
            return Some(String::from_utf8(data.to_vec()).unwrap());
        }
    }
    None
}

fn first_column(lines: &[String]) -> Vec<i64> {
    lines
        .iter()
        .map(|line| line.split(',').next().unwrap().parse().unwrap())
        .collect()
}

#[tokio::test]
async fn streams_every_row_once_in_key_order() {
    let (db, ctx) = setup().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    insert_posts(&db.db, ann.id, 1_201, "bulk").await;
    let router = router(&ctx);

    let mut body = export_body(&router, "/api/models/posts/export?sort=title&order=desc").await;
    let mut chunks = Vec::new();
    while let Some(chunk) = next_chunk(&mut body).await {
        chunks.push(chunk);
    }
    assert_eq!(
        chunks.len(),
        1 + 3,
        "header plus batches of 500, 500 and 201"
    );
    assert!(chunks[0].starts_with("id,title,"));

    let rows: Vec<String> = chunks[1..]
        .iter()
        .flat_map(|chunk| chunk.lines().map(ToString::to_string))
        .collect();
    let ids = first_column(&rows);
    assert_eq!(ids.len(), 1_201);
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "primary-key order, no repeats"
    );
}

#[tokio::test]
async fn pages_through_composite_keys() {
    let (db, ctx) = setup().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    let mut post_ids = Vec::new();
    for title in ["a", "b", "c"] {
        post_ids.push(create_post(&db.db, ann.id, title).await.id);
    }
    let rows: Vec<post_tags::ActiveModel> = post_ids
        .iter()
        .flat_map(|post_id| {
            (0..350).map(move |i| post_tags::ActiveModel {
                post_id: Set(*post_id),
                tag: Set(format!("t{i:04}")),
                weight: Set(1),
            })
        })
        .collect();
    for chunk in rows.chunks(500) {
        post_tags::Entity::insert_many(chunk.to_vec())
            .exec(&db.db)
            .await
            .expect("insert tags");
    }
    let router = router(&ctx);

    let mut body = export_body(&router, "/api/models/post_tags/export").await;
    let mut text = String::new();
    while let Some(chunk) = next_chunk(&mut body).await {
        text.push_str(&chunk);
    }
    let keys: Vec<(String, String)> = text
        .lines()
        .skip(1)
        .map(|line| {
            let mut parts = line.split(',');
            (
                parts.next().unwrap().to_string(),
                parts.next().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(keys.len(), 1_050);
    let mut sorted = keys.clone();
    sorted.sort_by(|a, b| {
        (a.0.parse::<i32>().unwrap(), &a.1).cmp(&(b.0.parse::<i32>().unwrap(), &b.1))
    });
    sorted.dedup();
    assert_eq!(keys, sorted, "composite-key order, no repeats");
}

#[tokio::test]
async fn reads_one_snapshot_while_rows_change() {
    if !is_postgres() {
        // The in-memory SQLite pool has a single connection, held by the export.
        return;
    }
    let (db, ctx) = setup().await;
    let ann = create_user(&db.db, "ann@example.com", true).await;
    insert_posts(&db.db, ann.id, 1_200, "before").await;
    let router = router(&ctx);

    let mut body = export_body(&router, "/api/models/posts/export").await;
    let header = next_chunk(&mut body).await.unwrap();
    assert!(header.starts_with("id,"));
    let first = next_chunk(&mut body).await.unwrap();
    assert_eq!(first.lines().count(), 500);

    // Change the table mid-export.
    posts::Entity::delete_many()
        .filter(posts::Column::Id.gt(first_column(&[first.lines().last().unwrap().into()])[0]))
        .exec(&db.db)
        .await
        .unwrap();
    insert_posts(&db.db, ann.id, 10, "after").await;

    let mut rest = String::new();
    while let Some(chunk) = next_chunk(&mut body).await {
        rest.push_str(&chunk);
    }
    assert_eq!(first.lines().count() + rest.lines().count(), 1_200);
    assert!(
        !rest.contains("after "),
        "rows added during the export are not included"
    );
}

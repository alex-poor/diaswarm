//! Does `open_bounded_store` actually apply its pragmas to pooled connections?
//!
//! **THIS TEST EXISTS BECAUSE THE ANSWER MATTERED AND I ASSUMED IT.** Capping
//! `cache_size` was measured as fixing the follower's memory growth on a
//! fifteen-minute window, and thirty minutes later the growth was back — so the
//! first question was whether the pragma reached the pool at all. It does:
//! every connection reports `-2000`. Which means the cap works and the page
//! cache was never the whole story, and that is worth knowing from a test
//! rather than re-deriving it on a phone at midnight.
#[tokio::test]
async fn the_cache_size_pragma_reaches_every_pooled_connection() {
    let dir = std::env::temp_dir().join(format!("pragma-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}", dir.join("t.sqlite").display());
    let store = diaswarm_keys::open_bounded_store(&url).await.unwrap();
    let pool = store.pool();
    for i in 0..4 {
        let v: i64 = sqlx::query_scalar("PRAGMA cache_size")
            .fetch_one(pool).await.unwrap();
        println!("connection {i}: cache_size = {v}");
        assert_eq!(v, -2000, "cache_size not applied to pooled connection {i}");
    }
}

//! A keys vault first migrated by the vendored-fork p2panda (0.7.x) recorded the
//! `create ordering` migration (20250928132846) with a checksum that upstream
//! p2panda-store 0.8.2 later changed — `orderer_ready`'s uniqueness went from a
//! `UNIQUE` table constraint to a named index, so the migration *bytes* differ
//! while the resulting schema is equivalent. sqlx's migrator checksums every
//! applied migration and rejects the whole open with `VersionMismatch`, so the
//! live publisher could not open its own vault after the 0.8.2 upgrade (observed
//! against the loop vault, 2026-11-26).
//!
//! [`open_bounded_store`] reconciles that one checksum before migrating. This
//! reproduces the exact failure on a facsimile vault and proves the recovery.

use diaswarm_keys::open_bounded_store;

const VERSION: i64 = 20250928132846;

/// Rewrite the create-ordering checksum to the fork-era value, turning a freshly
/// migrated 0.8.2 vault into a facsimile of the loop's live one. Static SQL (so no
/// `AssertSqlSafe` needed); the hex is the fork build's recorded checksum.
const SET_FORK_CHECKSUM: &str = "UPDATE _sqlx_migrations SET checksum = \
    X'39B1B06179F49D6424EDB6A21DC02B32C2850F3C6908D5EF5F9700702FB16DD146BFA04808B6AC62F9B45DFDC745DEE1' \
    WHERE version = 20250928132846";

fn tmp_url(tag: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!(
        "diaswarm-forkvault-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let url = format!("sqlite://{}", dir.join("keys.sqlite").display());
    (dir, url)
}

#[tokio::test]
async fn fork_era_vault_fails_the_raw_migrator_but_open_bounded_store_recovers_it() {
    let (_dir, url) = tmp_url("recover");

    // 1. A fresh open applies the 0.8.2 migrations with their correct checksums.
    drop(open_bounded_store(&url).await.expect("fresh open"));

    // 2. Make it look like a fork-era vault.
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let n = sqlx::query(SET_FORK_CHECKSUM).execute(&pool).await.unwrap().rows_affected();
    assert_eq!(n, 1, "expected to rewrite exactly one migration row");

    // 3. The RAW migrator rejects it — this is precisely the bug the shim exists
    //    for, so assert the specific failure rather than just \"an error\".
    let raw = p2panda_store::sqlite::run_pending_migrations(&pool).await;
    pool.close().await;
    assert!(
        matches!(
            raw,
            Err(p2panda_store::SqliteError::Migrate(
                sqlx::migrate::MigrateError::VersionMismatch(VERSION)
            ))
        ),
        "expected VersionMismatch({VERSION}) from the raw migrator, got {raw:?}"
    );

    // 4. open_bounded_store reconciles the checksum and opens cleanly.
    drop(open_bounded_store(&url).await.expect("fork-era vault must open after reconcile"));

    // 5. Idempotent: a second open is still fine (the row now matches upstream).
    drop(open_bounded_store(&url).await.expect("second open"));
}

#[tokio::test]
async fn reconcile_never_touches_an_unforeseen_checksum() {
    let (_dir, url) = tmp_url("untouched");
    drop(open_bounded_store(&url).await.expect("fresh open"));

    // A checksum that is neither the fork value nor upstream's: the shim must
    // leave it alone, so the raw migrator still rejects it. This guards against a
    // shim that blindly rewrites whatever it finds.
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = X'DEADBEEF' WHERE version = 20250928132846",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let reopened = open_bounded_store(&url).await;
    assert!(
        matches!(
            reopened,
            Err(p2panda_store::SqliteError::Migrate(
                sqlx::migrate::MigrateError::VersionMismatch(VERSION)
            ))
        ),
        "an unforeseen checksum must still be rejected, got {reopened:?}"
    );
}

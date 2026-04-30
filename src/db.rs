//! Tiny query helpers; the schema lives in `migrations/`.

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub async fn count(pool: &SqlitePool, sql: &str) -> Result<i64> {
    let row: (i64,) = sqlx::query_as(sql).fetch_one(pool).await?;
    Ok(row.0)
}

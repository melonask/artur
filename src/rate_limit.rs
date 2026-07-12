use crate::{
    config::{RateLimitConfig, StoreConfig, StoreDriver},
    error::{ArturError, Result},
    store::{postgres_client, sqlite_path},
};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct RateLimitResult {
    pub allowed: bool,
    pub remaining: u64,
    pub retry_after: u64,
}

pub async fn check(
    endpoint: &str,
    key: &str,
    config: &RateLimitConfig,
    store: &StoreConfig,
) -> Result<RateLimitResult> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ArturError::Store("system clock is before Unix epoch".to_string()))?
        .as_secs();
    let window = now / config.window_secs * config.window_secs;
    let retry_after = window + config.window_secs - now;
    let bounded_key = Sha256::digest(format!("{endpoint}\0{key}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    match store.driver {
        StoreDriver::Sqlite => {
            sqlite_check(
                sqlite_path(&store.url)?,
                endpoint.to_string(),
                bounded_key,
                window as i64,
                config.requests as i64,
                retry_after,
            )
            .await
        }
        StoreDriver::Postgres => {
            postgres_check(
                &store.url,
                endpoint,
                &bounded_key,
                window as i64,
                config.requests as i64,
                retry_after,
            )
            .await
        }
    }
}

async fn sqlite_check(
    path: std::path::PathBuf,
    endpoint: String,
    key: String,
    window: i64,
    quota: i64,
    retry_after: u64,
) -> Result<RateLimitResult> {
    tokio::task::spawn_blocking(move || -> Result<_> {
        if let Some(parent) = path.parent() && !parent.as_os_str().is_empty() { std::fs::create_dir_all(parent)?; }
        let mut conn = rusqlite::Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS artur_rate_limits (endpoint TEXT NOT NULL, key_hash TEXT NOT NULL, window_start INTEGER NOT NULL, count INTEGER NOT NULL, PRIMARY KEY(endpoint,key_hash,window_start)); CREATE INDEX IF NOT EXISTS artur_rate_limits_expiry ON artur_rate_limits(window_start);")?;
        tx.execute("DELETE FROM artur_rate_limits WHERE window_start < ?1", [window - 172_800])?;
        let count: i64 = tx.query_row("SELECT count FROM artur_rate_limits WHERE endpoint=?1 AND key_hash=?2 AND window_start=?3", rusqlite::params![endpoint, key, window], |r| r.get(0)).unwrap_or(0);
        let allowed = count < quota;
        let new_count = if allowed { count + 1 } else { count };
        if allowed { tx.execute("INSERT INTO artur_rate_limits(endpoint,key_hash,window_start,count) VALUES(?1,?2,?3,1) ON CONFLICT(endpoint,key_hash,window_start) DO UPDATE SET count=count+1", rusqlite::params![endpoint,key,window])?; }
        tx.commit()?;
        Ok(RateLimitResult { allowed, remaining: quota.saturating_sub(new_count) as u64, retry_after })
    }).await.map_err(|e| ArturError::Store(format!("rate limit task join error: {e}")))?
}

async fn postgres_check(
    url: &str,
    endpoint: &str,
    key: &str,
    window: i64,
    quota: i64,
    retry_after: u64,
) -> Result<RateLimitResult> {
    let mut client = postgres_client(url).await?;
    let tx = client
        .transaction()
        .await
        .map_err(|e| ArturError::Store(format!("postgres rate limit transaction failed: {e}")))?;
    tx.batch_execute("CREATE TABLE IF NOT EXISTS artur_rate_limits (endpoint TEXT NOT NULL, key_hash TEXT NOT NULL, window_start BIGINT NOT NULL, count BIGINT NOT NULL, PRIMARY KEY(endpoint,key_hash,window_start)); CREATE INDEX IF NOT EXISTS artur_rate_limits_expiry ON artur_rate_limits(window_start);").await.map_err(|e| ArturError::Store(format!("postgres rate limit schema failed: {e}")))?;
    tx.execute(
        "DELETE FROM artur_rate_limits WHERE window_start < $1",
        &[&(window - 172_800)],
    )
    .await
    .map_err(|e| ArturError::Store(format!("postgres rate limit cleanup failed: {e}")))?;
    let row = tx.query_opt("INSERT INTO artur_rate_limits(endpoint,key_hash,window_start,count) VALUES($1,$2,$3,1) ON CONFLICT(endpoint,key_hash,window_start) DO UPDATE SET count=artur_rate_limits.count+1 WHERE artur_rate_limits.count < $4 RETURNING count", &[&endpoint, &key, &window, &quota]).await.map_err(|e| ArturError::Store(format!("postgres rate limit increment failed: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| ArturError::Store(format!("postgres rate limit commit failed: {e}")))?;
    let count = row.map(|r| r.get::<_, i64>(0));
    Ok(RateLimitResult {
        allowed: count.is_some(),
        remaining: count.map(|c| quota.saturating_sub(c) as u64).unwrap_or(0),
        retry_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{StoreConfig, StoreDriver};

    fn sqlite_store(file: &std::path::Path) -> StoreConfig {
        StoreConfig {
            driver: StoreDriver::Sqlite,
            url: format!("sqlite://{}", file.display()),
            connect_timeout_secs: None,
        }
    }
    fn limit() -> RateLimitConfig {
        RateLimitConfig {
            store: "rate".into(),
            key: "{{client.ip}}".into(),
            requests: 2,
            window_secs: 60,
        }
    }

    #[tokio::test]
    async fn sqlite_rate_limit_allows_quota_then_denies_with_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = sqlite_store(&dir.path().join("rate.db"));
        let cfg = limit();
        let first = check("upload", "192.0.2.1", &cfg, &store).await.unwrap();
        let second = check("upload", "192.0.2.1", &cfg, &store).await.unwrap();
        let denied = check("upload", "192.0.2.1", &cfg, &store).await.unwrap();
        assert!(first.allowed);
        assert_eq!(first.remaining, 1);
        assert!(second.allowed);
        assert_eq!(second.remaining, 0);
        assert!(!denied.allowed);
        assert!(denied.retry_after > 0);
    }

    #[tokio::test]
    async fn sqlite_rate_limit_scopes_keys_and_endpoints_independently() {
        let dir = tempfile::tempdir().unwrap();
        let store = sqlite_store(&dir.path().join("rate.db"));
        let cfg = limit();
        for _ in 0..2 {
            assert!(check("one", "a", &cfg, &store).await.unwrap().allowed);
        }
        assert!(!check("one", "a", &cfg, &store).await.unwrap().allowed);
        assert!(check("one", "b", &cfg, &store).await.unwrap().allowed);
        assert!(check("two", "a", &cfg, &store).await.unwrap().allowed);
    }

    #[tokio::test]
    #[ignore = "requires ARTUR_POSTGRES_URL pointing to a disposable PostgreSQL database"]
    async fn postgres_rate_limit_allows_quota_then_denies() {
        let store = StoreConfig {
            driver: StoreDriver::Postgres,
            url: std::env::var("ARTUR_POSTGRES_URL").expect("ARTUR_POSTGRES_URL is required"),
            connect_timeout_secs: None,
        };
        let cfg = RateLimitConfig {
            requests: 2,
            ..limit()
        };
        let key = uuid::Uuid::new_v4().to_string();
        assert!(
            check("postgres-rate-test", &key, &cfg, &store)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            check("postgres-rate-test", &key, &cfg, &store)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            !check("postgres-rate-test", &key, &cfg, &store)
                .await
                .unwrap()
                .allowed
        );
    }
}

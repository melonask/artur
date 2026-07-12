use crate::{
    config::{IdempotencyConfig, StoreConfig, StoreDriver},
    error::{ArturError, Result},
    process::RequestContext,
    store::{postgres_client, sqlite_path},
};
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_KEY_BYTES: usize = 255;
const DEFAULT_SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
type Record = (String, i64, Option<i64>, Option<Vec<u8>>, Option<Vec<u8>>);
pub enum Claim {
    Claimed,
    Replay(StoredResponse),
}
#[derive(Serialize, Deserialize)]
pub struct StoredResponse {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, Vec<u8>)>,
}

pub fn key(headers: &HeaderMap, config: &IdempotencyConfig) -> Result<Option<String>> {
    let name = HeaderName::from_bytes(config.header.as_bytes())
        .map_err(|_| ArturError::Config("invalid idempotency header".to_string()))?;
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(ArturError::Request(
            "multiple Idempotency-Key header values are not allowed".to_string(),
        ));
    }
    let key = value
        .to_str()
        .map_err(|_| ArturError::Request("Idempotency-Key must be valid ASCII".to_string()))?;
    if key.is_empty()
        || key.len() > MAX_KEY_BYTES
        || key.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ArturError::Request(format!(
            "Idempotency-Key must be between 1 and {MAX_KEY_BYTES} non-control ASCII bytes"
        )));
    }
    Ok(Some(key.to_string()))
}

pub async fn claim(
    endpoint: &str,
    key: &str,
    request: &RequestContext,
    config: &IdempotencyConfig,
    store: &StoreConfig,
) -> Result<Claim> {
    match store.driver {
        StoreDriver::Sqlite => claim_sqlite(endpoint, key, request, config, store).await,
        StoreDriver::Postgres => claim_postgres(endpoint, key, request, config, store).await,
    }
}

async fn claim_sqlite(
    endpoint: &str,
    key: &str,
    request: &RequestContext,
    config: &IdempotencyConfig,
    store: &StoreConfig,
) -> Result<Claim> {
    let path = sqlite_path(&store.url)?;
    let endpoint = endpoint.to_string();
    let key = key.to_string();
    let fingerprint = fingerprint(&endpoint, request);
    let ttl = config.ttl_secs;
    let busy_timeout_secs = store.connect_timeout_secs;
    tokio::task::spawn_blocking(move || {
        let mut connection = open_sqlite(&path, busy_timeout_secs)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS artur_idempotency (endpoint_name TEXT NOT NULL, key TEXT NOT NULL, fingerprint TEXT NOT NULL, state INTEGER NOT NULL, status INTEGER, body BLOB, headers BLOB, expires_at INTEGER NOT NULL, PRIMARY KEY (endpoint_name, key)); CREATE INDEX IF NOT EXISTS artur_idempotency_expiry ON artur_idempotency (expires_at);")?;
        let now = unix_seconds()?; transaction.execute("DELETE FROM artur_idempotency WHERE expires_at <= ?1", [now])?;
        let existing: Option<Record> = transaction.query_row("SELECT fingerprint, state, status, body, headers FROM artur_idempotency WHERE endpoint_name = ?1 AND key = ?2", params![endpoint, key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).optional()?;
        let result = match existing {
            Some((old, _, _, _, _)) if old != fingerprint => return Err(ArturError::IdempotencyMismatch("Idempotency-Key is already associated with a different request".to_string())),
            Some((_, 0, _, _, _)) => return Err(ArturError::IdempotencyConflict("a request with this Idempotency-Key is still processing".to_string())),
            Some((_, 1, status, body, headers)) => Claim::Replay(StoredResponse { status: status.ok_or_else(|| ArturError::Store("idempotency record has no status".to_string()))? as u16, body: body.ok_or_else(|| ArturError::Store("idempotency record has no body".to_string()))?, headers: serde_json::from_slice(&headers.ok_or_else(|| ArturError::Store("idempotency record has no headers".to_string()))?)? }),
            Some(_) => return Err(ArturError::Store("invalid idempotency record state".to_string())),
            None => { transaction.execute("INSERT INTO artur_idempotency (endpoint_name, key, fingerprint, state, expires_at) VALUES (?1, ?2, ?3, 0, ?4)", params![endpoint, key, fingerprint, now.saturating_add(ttl as i64)])?; Claim::Claimed }
        };
        transaction.commit()?; Ok(result)
    }).await.map_err(|error| ArturError::Store(format!("idempotency task join error: {error}")))?
}

pub async fn complete(
    endpoint: &str,
    key: &str,
    store: &StoreConfig,
    response: StoredResponse,
) -> Result<()> {
    match store.driver {
        StoreDriver::Sqlite => complete_sqlite(endpoint, key, store, response).await,
        StoreDriver::Postgres => complete_postgres(endpoint, key, store, response).await,
    }
}

async fn complete_sqlite(
    endpoint: &str,
    key: &str,
    store: &StoreConfig,
    response: StoredResponse,
) -> Result<()> {
    let path = sqlite_path(&store.url)?;
    let endpoint = endpoint.to_string();
    let key = key.to_string();
    let busy_timeout_secs = store.connect_timeout_secs;
    tokio::task::spawn_blocking(move || {
        let connection = open_sqlite(&path, busy_timeout_secs)?;
        let changed = connection.execute(
            "UPDATE artur_idempotency SET state = 1, status = ?1, body = ?2, headers = ?3 WHERE endpoint_name = ?4 AND key = ?5 AND state = 0",
            params![i64::from(response.status), response.body, serde_json::to_vec(&response.headers)?, endpoint, key],
        )?;
        if changed != 1 {
            return Err(ArturError::Store("idempotency reservation was not available for completion".to_string()));
        }
        Ok(())
    }).await.map_err(|error| ArturError::Store(format!("idempotency task join error: {error}")))?
}
pub async fn release(endpoint: &str, key: &str, store: &StoreConfig) -> Result<()> {
    match store.driver {
        StoreDriver::Sqlite => release_sqlite(endpoint, key, store).await,
        StoreDriver::Postgres => release_postgres(endpoint, key, store).await,
    }
}
async fn release_sqlite(endpoint: &str, key: &str, store: &StoreConfig) -> Result<()> {
    let path = sqlite_path(&store.url)?;
    let endpoint = endpoint.to_string();
    let key = key.to_string();
    let busy_timeout_secs = store.connect_timeout_secs;
    tokio::task::spawn_blocking(move || {
        open_sqlite(&path, busy_timeout_secs)?.execute(
            "DELETE FROM artur_idempotency WHERE endpoint_name = ?1 AND key = ?2 AND state = 0",
            params![endpoint, key],
        )?;
        Ok(())
    })
    .await
    .map_err(|error| ArturError::Store(format!("idempotency task join error: {error}")))?
}

/// Opens an idempotency database with a wait policy for concurrent writers.
fn open_sqlite(path: &std::path::Path, busy_timeout_secs: Option<u64>) -> Result<Connection> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(
        busy_timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_SQLITE_BUSY_TIMEOUT),
    )?;
    Ok(connection)
}

async fn claim_postgres(
    endpoint: &str,
    key: &str,
    request: &RequestContext,
    config: &IdempotencyConfig,
    store: &StoreConfig,
) -> Result<Claim> {
    let mut client = postgres_client(&store.url).await?;
    let transaction = client.transaction().await.map_err(postgres_error)?;
    transaction.batch_execute("CREATE TABLE IF NOT EXISTS artur_idempotency (endpoint_name TEXT NOT NULL, key TEXT NOT NULL, fingerprint TEXT NOT NULL, state SMALLINT NOT NULL, status INTEGER, body BYTEA, headers BYTEA, expires_at BIGINT NOT NULL, PRIMARY KEY (endpoint_name, key)); CREATE INDEX IF NOT EXISTS artur_idempotency_expiry ON artur_idempotency (expires_at);").await.map_err(postgres_error)?;
    transaction
        .execute(
            "SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))",
            &[&endpoint, &key],
        )
        .await
        .map_err(postgres_error)?;
    transaction
        .execute(
            "DELETE FROM artur_idempotency WHERE expires_at <= EXTRACT(EPOCH FROM NOW())::BIGINT",
            &[],
        )
        .await
        .map_err(postgres_error)?;
    let fingerprint = fingerprint(endpoint, request);
    let record = transaction.query_opt("SELECT fingerprint, state, status, body, headers FROM artur_idempotency WHERE endpoint_name = $1 AND key = $2 FOR UPDATE", &[&endpoint, &key]).await.map_err(postgres_error)?;
    let claim = if let Some(record) = record {
        let existing: String = record.try_get(0).map_err(postgres_error)?;
        if existing != fingerprint {
            return Err(ArturError::IdempotencyMismatch(
                "Idempotency-Key is already associated with a different request".to_string(),
            ));
        }
        match record.try_get::<_, i16>(1).map_err(postgres_error)? {
            0 => {
                return Err(ArturError::IdempotencyConflict(
                    "a request with this Idempotency-Key is still processing".to_string(),
                ));
            }
            1 => Claim::Replay(StoredResponse {
                status: record.try_get::<_, i32>(2).map_err(postgres_error)? as u16,
                body: record.try_get(3).map_err(postgres_error)?,
                headers: serde_json::from_slice(
                    &record.try_get::<_, Vec<u8>>(4).map_err(postgres_error)?,
                )?,
            }),
            _ => {
                return Err(ArturError::Store(
                    "invalid idempotency record state".to_string(),
                ));
            }
        }
    } else {
        transaction.execute("INSERT INTO artur_idempotency (endpoint_name, key, fingerprint, state, expires_at) VALUES ($1, $2, $3, 0, EXTRACT(EPOCH FROM NOW())::BIGINT + $4)", &[&endpoint, &key, &fingerprint, &(config.ttl_secs as i64)]).await.map_err(postgres_error)?;
        Claim::Claimed
    };
    transaction.commit().await.map_err(postgres_error)?;
    Ok(claim)
}

async fn complete_postgres(
    endpoint: &str,
    key: &str,
    store: &StoreConfig,
    response: StoredResponse,
) -> Result<()> {
    let client = postgres_client(&store.url).await?;
    let headers = serde_json::to_vec(&response.headers)?;
    let changed = client.execute("UPDATE artur_idempotency SET state = 1, status = $1, body = $2, headers = $3 WHERE endpoint_name = $4 AND key = $5 AND state = 0", &[&(response.status as i32), &response.body, &headers, &endpoint, &key]).await.map_err(postgres_error)?;
    if changed != 1 {
        return Err(ArturError::Store(
            "idempotency reservation was not available for completion".to_string(),
        ));
    }
    Ok(())
}

async fn release_postgres(endpoint: &str, key: &str, store: &StoreConfig) -> Result<()> {
    postgres_client(&store.url)
        .await?
        .execute(
            "DELETE FROM artur_idempotency WHERE endpoint_name = $1 AND key = $2 AND state = 0",
            &[&endpoint, &key],
        )
        .await
        .map_err(postgres_error)?;
    Ok(())
}

fn postgres_error(error: tokio_postgres::Error) -> ArturError {
    ArturError::Store(format!("postgres operation failed: {error}"))
}
pub async fn capture(response: Response, max: usize) -> Result<(Response, StoredResponse)> {
    let (parts, body) = response.into_parts();
    let body = to_bytes(body, max).await.map_err(|error| {
        ArturError::Store(format!(
            "idempotency response exceeds max_response_bytes: {error}"
        ))
    })?;
    let stored = StoredResponse {
        status: parts.status.as_u16(),
        body: body.to_vec(),
        headers: parts
            .headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
            .collect(),
    };
    Ok((Response::from_parts(parts, Body::from(body)), stored))
}
pub fn replay(response: StoredResponse) -> Result<Response> {
    let mut builder =
        Response::builder().status(StatusCode::from_u16(response.status).map_err(|error| {
            ArturError::Store(format!("idempotency record has invalid status: {error}"))
        })?);
    let headers = builder
        .headers_mut()
        .ok_or_else(|| ArturError::Store("unable to build idempotency response".to_string()))?;
    for (name, value) in response.headers {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                ArturError::Store(format!(
                    "idempotency record has invalid header name: {error}"
                ))
            })?,
            HeaderValue::from_bytes(&value).map_err(|error| {
                ArturError::Store(format!(
                    "idempotency record has invalid header value: {error}"
                ))
            })?,
        );
    }
    builder.body(Body::from(response.body)).map_err(|error| {
        ArturError::Store(format!("unable to replay idempotency response: {error}"))
    })
}
fn fingerprint(endpoint: &str, request: &RequestContext) -> String {
    let mut hash = Sha256::new();
    for value in [
        endpoint.as_bytes(),
        request.method.as_bytes(),
        request.path.as_bytes(),
        &serde_json::to_vec(&request.params).expect("serializable"),
        &serde_json::to_vec(&request.query).expect("serializable"),
        &request.body_bytes,
    ] {
        hash.update(value);
        hash.update([0]);
    }
    let mut fingerprint = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(&mut fingerprint, "{byte:02x}").expect("writing to String cannot fail");
    }
    fingerprint
}
fn unix_seconds() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ArturError::Store(format!("system clock is before Unix epoch: {error}")))?
        .as_secs() as i64)
}

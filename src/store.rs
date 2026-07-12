use crate::{
    config::{StoreConfig, StoreDriver, WorkflowStepConfig, WorkflowStepKind},
    error::{ArturError, Result},
    process::{RequestContext, render_template},
};
use rusqlite::{Connection, params_from_iter, types::ValueRef};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::PathBuf;
use tokio_postgres::{Client, NoTls, Row, types::Type};

#[derive(Debug, Clone, Serialize)]
pub struct StoreOutput {
    pub ok: bool,
    pub store: String,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Value>>,
}

pub async fn run_store_step(
    store_name: &str,
    store: &StoreConfig,
    step: &WorkflowStepConfig,
    request: &RequestContext,
) -> Result<StoreOutput> {
    match store.driver {
        StoreDriver::Sqlite => run_sqlite_step(store_name, store, step, request).await,
        StoreDriver::Postgres => run_postgres_step(store_name, store, step, request).await,
    }
}

async fn run_postgres_step(
    store_name: &str,
    store: &StoreConfig,
    step: &WorkflowStepConfig,
    request: &RequestContext,
) -> Result<StoreOutput> {
    let sql = render_template(
        step.sql
            .as_deref()
            .ok_or_else(|| ArturError::Config(format!("store step {} is missing sql", step.id)))?,
        request,
    )?;
    let params = step
        .params
        .iter()
        .map(|param| render_template(param, request))
        .collect::<Result<Vec<_>>>()?;
    let client = postgres_client(&store.url).await?;
    let values = params
        .iter()
        .map(|value| value as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect::<Vec<_>>();
    match step.kind {
        WorkflowStepKind::StoreQuery => {
            let rows = client.query(&sql, &values).await.map_err(postgres_error)?;
            Ok(StoreOutput {
                ok: true,
                store: store_name.to_string(),
                operation: "query".to_string(),
                rows_affected: None,
                rows: Some(
                    rows.iter()
                        .map(postgres_row_to_json)
                        .collect::<Result<_>>()?,
                ),
            })
        }
        WorkflowStepKind::StoreExecute => {
            let rows_affected = client
                .execute(&sql, &values)
                .await
                .map_err(postgres_error)?;
            Ok(StoreOutput {
                ok: true,
                store: store_name.to_string(),
                operation: "execute".to_string(),
                rows_affected: Some(rows_affected as usize),
                rows: None,
            })
        }
        _ => Err(ArturError::Config(format!(
            "store step {} has invalid type",
            step.id
        ))),
    }
}

pub(crate) async fn postgres_client(url: &str) -> Result<Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .map_err(postgres_error)?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::error!(%error, "postgres connection terminated");
        }
    });
    Ok(client)
}

fn postgres_error(error: tokio_postgres::Error) -> ArturError {
    ArturError::Store(format!("postgres operation failed: {error}"))
}

fn postgres_row_to_json(row: &Row) -> Result<Value> {
    let mut object = Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        let value = match *column.type_() {
            Type::BOOL => Value::Bool(row.try_get(index).map_err(postgres_error)?),
            Type::INT2 => Value::Number(
                i64::from(row.try_get::<_, i16>(index).map_err(postgres_error)?).into(),
            ),
            Type::INT4 => Value::Number(
                i64::from(row.try_get::<_, i32>(index).map_err(postgres_error)?).into(),
            ),
            Type::INT8 => {
                Value::Number(row.try_get::<_, i64>(index).map_err(postgres_error)?.into())
            }
            Type::FLOAT4 => serde_json::Number::from_f64(f64::from(
                row.try_get::<_, f32>(index).map_err(postgres_error)?,
            ))
            .map(Value::Number)
            .unwrap_or(Value::Null),
            Type::FLOAT8 => {
                serde_json::Number::from_f64(row.try_get::<_, f64>(index).map_err(postgres_error)?)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            }
            Type::JSON | Type::JSONB => row.try_get(index).map_err(postgres_error)?,
            Type::BYTEA => Value::String(format!(
                "<{} byte blob>",
                row.try_get::<_, Vec<u8>>(index)
                    .map_err(postgres_error)?
                    .len()
            )),
            _ => Value::String(row.try_get::<_, String>(index).map_err(postgres_error)?),
        };
        object.insert(column.name().to_string(), value);
    }
    Ok(Value::Object(object))
}

async fn run_sqlite_step(
    store_name: &str,
    store: &StoreConfig,
    step: &WorkflowStepConfig,
    request: &RequestContext,
) -> Result<StoreOutput> {
    let path = sqlite_path(&store.url)?;
    let sql = render_template(
        step.sql
            .as_deref()
            .ok_or_else(|| ArturError::Config(format!("store step {} is missing sql", step.id)))?,
        request,
    )?;
    let params = step
        .params
        .iter()
        .map(|param| render_template(param, request))
        .collect::<Result<Vec<_>>>()?;
    let operation = match step.kind {
        WorkflowStepKind::StoreQuery => "query",
        WorkflowStepKind::StoreExecute => "execute",
        WorkflowStepKind::Task | WorkflowStepKind::HttpRequest | WorkflowStepKind::Respond => {
            "unknown"
        }
    }
    .to_string();
    let store_name = store_name.to_string();

    tokio::task::spawn_blocking(move || -> Result<StoreOutput> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        if operation == "query" {
            query_rows(&conn, &store_name, &operation, &sql, &params)
        } else {
            let rows_affected = conn.execute(&sql, params_from_iter(params.iter()))?;
            Ok(StoreOutput {
                ok: true,
                store: store_name,
                operation,
                rows_affected: Some(rows_affected),
                rows: None,
            })
        }
    })
    .await
    .map_err(|err| ArturError::Store(format!("store task join error: {err}")))?
}

fn query_rows(
    conn: &Connection,
    store_name: &str,
    operation: &str,
    sql: &str,
    params: &[String],
) -> Result<StoreOutput> {
    let mut stmt = conn.prepare(sql)?;
    let column_names = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let mut rows = stmt.query(params_from_iter(params.iter()))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut object = Map::new();
        for (index, name) in column_names.iter().enumerate() {
            object.insert(name.clone(), sqlite_value_to_json(row.get_ref(index)?));
        }
        out.push(Value::Object(object));
    }
    Ok(StoreOutput {
        ok: true,
        store: store_name.to_string(),
        operation: operation.to_string(),
        rows_affected: None,
        rows: Some(out),
    })
}

pub(crate) fn sqlite_path(url: &str) -> Result<PathBuf> {
    if url == ":memory:" {
        return Ok(PathBuf::from(url));
    }
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    if path.trim().is_empty() {
        return Err(ArturError::Config(
            "sqlite store url cannot be empty".to_string(),
        ));
    }
    Ok(PathBuf::from(path))
}

fn sqlite_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::Number(value.into()),
        ValueRef::Real(value) => serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).to_string()),
        ValueRef::Blob(value) => Value::String(format!("<{} byte blob>", value.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{StoreDriver, WorkflowStepConfig};
    use bytes::Bytes;
    use std::collections::BTreeMap;

    #[test]
    fn parses_sqlite_url_to_path() {
        assert_eq!(
            sqlite_path("sqlite://data/app.db").unwrap(),
            PathBuf::from("data/app.db")
        );
        assert_eq!(
            sqlite_path("sqlite:/tmp/app.db").unwrap(),
            PathBuf::from("/tmp/app.db")
        );
    }

    #[test]
    fn rejects_empty_sqlite_url() {
        let err = sqlite_path("sqlite://").unwrap_err().to_string();
        assert!(err.contains("sqlite store url cannot be empty"));
    }

    #[test]
    fn postgres_driver_is_configurable() {
        let store = StoreConfig {
            driver: StoreDriver::Postgres,
            url: "postgres://localhost/app".to_string(),
            connect_timeout_secs: None,
        };
        assert_eq!(store.driver, StoreDriver::Postgres);
    }

    #[tokio::test]
    #[ignore = "requires ARTUR_POSTGRES_URL pointing to a disposable PostgreSQL database"]
    async fn postgres_query_and_execute_use_bound_parameters() {
        let store = StoreConfig {
            driver: StoreDriver::Postgres,
            url: std::env::var("ARTUR_POSTGRES_URL").expect("ARTUR_POSTGRES_URL is required"),
            connect_timeout_secs: None,
        };
        let table = format!("artur_store_test_{}", uuid::Uuid::new_v4().simple());
        let request = RequestContext::from_parts(
            "POST".to_string(),
            "/".to_string(),
            "/".to_string(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            Bytes::new(),
        );
        let execute = |sql: String, params: Vec<String>| WorkflowStepConfig {
            id: "execute".to_string(),
            kind: WorkflowStepKind::StoreExecute,
            depends_on: vec![],
            task: None,
            store: Some("pg".to_string()),
            sql: Some(sql),
            params,
            transport: None,
            url: None,
            method: None,
            headers: BTreeMap::new(),
            body: Value::Null,
            timeout_ms: None,
            value: Value::Null,
            continue_on_error: false,
        };
        run_store_step(
            "pg",
            &store,
            &execute(
                format!("CREATE TABLE {table} (value TEXT NOT NULL)"),
                vec![],
            ),
            &request,
        )
        .await
        .unwrap();
        run_store_step(
            "pg",
            &store,
            &execute(
                format!("INSERT INTO {table} (value) VALUES ($1)"),
                vec!["safe value".to_string()],
            ),
            &request,
        )
        .await
        .unwrap();
        let query = WorkflowStepConfig {
            kind: WorkflowStepKind::StoreQuery,
            ..execute(
                format!("SELECT value FROM {table} WHERE value = $1"),
                vec!["safe value".to_string()],
            )
        };
        let output = run_store_step("pg", &store, &query, &request)
            .await
            .unwrap();
        assert_eq!(
            output.rows.unwrap(),
            vec![serde_json::json!({ "value": "safe value" })]
        );
        run_store_step(
            "pg",
            &store,
            &execute(format!("DROP TABLE {table}"), vec![]),
            &request,
        )
        .await
        .unwrap();
    }
}

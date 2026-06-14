use crate::{
    config::{StoreConfig, StoreDriver, WorkflowStepConfig, WorkflowStepKind},
    error::{ArturError, Result},
    process::{RequestContext, render_template},
};
use rusqlite::{Connection, params_from_iter, types::ValueRef};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::PathBuf;

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
        StoreDriver::Postgres => Err(ArturError::Store(format!(
            "built-in store execution currently supports sqlite; store {store_name} is postgres. Use a Bria task or a package-local process for this step."
        ))),
    }
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

fn sqlite_path(url: &str) -> Result<PathBuf> {
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
    use crate::config::StoreDriver;

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
    fn postgres_driver_is_configurable_even_if_builtin_executor_rejects_it() {
        let store = StoreConfig {
            driver: StoreDriver::Postgres,
            url: "postgres://localhost/app".to_string(),
            migrate: false,
            connect_timeout_secs: None,
            max_connections: None,
        };
        assert_eq!(store.driver, StoreDriver::Postgres);
    }
}

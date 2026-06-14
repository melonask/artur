use artur::{config::*, load_config, server::build_router};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

fn test_config() -> AppConfig {
    let endpoints = vec![
        EndpointConfig {
            name: "hello".to_string(),
            method: HttpMethod::Get,
            path: "/hello".to_string(),
            action: EndpointAction::RespondStatic,
            task: None,
            response: Some(StaticResponseConfig {
                status: 200,
                body: serde_json::json!({ "ok": true, "name": "artur" }),
                headers: Default::default(),
            }),
            security: EndpointSecurityConfig::default(),
            body_limit_bytes: None,
            steps: vec![],
            result: WorkflowResponseConfig::default(),
        },
        EndpointConfig {
            name: "echo".to_string(),
            method: HttpMethod::Post,
            path: "/echo/{id}".to_string(),
            action: EndpointAction::TaskRun,
            task: Some("cat_json".to_string()),
            response: None,
            security: EndpointSecurityConfig::default(),
            body_limit_bytes: None,
            steps: vec![],
            result: WorkflowResponseConfig::default(),
        },
        EndpointConfig {
            name: "async".to_string(),
            method: HttpMethod::Post,
            path: "/async".to_string(),
            action: EndpointAction::TaskRun,
            task: Some("async_json".to_string()),
            response: None,
            security: EndpointSecurityConfig::default(),
            body_limit_bytes: None,
            steps: vec![],
            result: WorkflowResponseConfig::default(),
        },
        EndpointConfig {
            name: "job".to_string(),
            method: HttpMethod::Get,
            path: "/jobs/{job_id}".to_string(),
            action: EndpointAction::JobGet,
            task: None,
            response: None,
            security: EndpointSecurityConfig::default(),
            body_limit_bytes: None,
            steps: vec![],
            result: WorkflowResponseConfig::default(),
        },
    ];
    let tasks = vec![
        TaskConfig {
            name: "cat_json".to_string(),
            mode: TaskMode::Sync,
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "cat".to_string()],
            env: Default::default(),
            working_dir: None,
            inherit_env: true,
            success_exit_codes: vec![0],
            timeout_ms: 5000,
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 1024 * 1024,
            stdin: TaskStdin::Body,
            stdout_format: TaskOutputFormat::Json,
        },
        TaskConfig {
            name: "async_json".to_string(),
            mode: TaskMode::Async,
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "printf '{\"ok\":true}'".to_string()],
            env: Default::default(),
            working_dir: None,
            inherit_env: true,
            success_exit_codes: vec![0],
            timeout_ms: 5000,
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 1024 * 1024,
            stdin: TaskStdin::None,
            stdout_format: TaskOutputFormat::Json,
        },
    ];
    AppConfig {
        version: 1,
        artur: ArturConfig {
            endpoints,
            tasks,
            ..ArturConfig::default()
        },
        ..AppConfig::default()
    }
}

#[tokio::test]
async fn static_endpoint_returns_configured_json() {
    let app = build_router(test_config()).await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/hello")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["name"], "artur");
}

#[tokio::test]
async fn process_endpoint_can_read_body_and_return_parsed_json() {
    let app = build_router(test_config()).await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo/abc?source=test")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"hello":"world"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["task"], "cat_json");
    assert_eq!(value["json"]["hello"], "world");
}

#[tokio::test]
async fn async_process_endpoint_returns_job_and_job_result() {
    let app = build_router(test_config()).await.unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/async")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let accepted: Value = serde_json::from_slice(&bytes).unwrap();
    let job_id = accepted["job_id"].as_str().unwrap();

    let mut last_status = String::new();
    for _ in 0..20 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/jobs/{job_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let job: Value = serde_json::from_slice(&bytes).unwrap();
        last_status = job["status"].as_str().unwrap().to_string();
        if last_status == "completed" {
            assert_eq!(job["result"]["json"]["ok"], true);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job did not complete; last status was {last_status}");
}

#[tokio::test]
async fn workflow_can_run_task_write_sqlite_and_return_combined_rows() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("artur.sqlite3");
    let config_path = temp_dir.path().join("Config.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"
version = 1

[stores.artur]
driver = "sqlite"
url = "sqlite://{}"

[[artur.endpoints]]
name = "create_space"
method = "POST"
path = "/spaces"
action = "workflow.run"

[artur.endpoints.result]
include_steps = false
body = {{ sid = "{{{{steps.lookup.rows.0.sid}}}}", symbol = "{{{{steps.lookup.rows.0.symbol}}}}" }}

[[artur.endpoints.steps]]
id = "schema"
type = "store.execute"
store = "artur"
sql = "CREATE TABLE IF NOT EXISTS spaces (sid TEXT PRIMARY KEY, symbol TEXT NOT NULL)"

[[artur.endpoints.steps]]
id = "sid"
type = "task"
task = "sid_create"

[[artur.endpoints.steps]]
id = "insert"
type = "store.execute"
store = "artur"
depends_on = ["schema", "sid"]
sql = "INSERT INTO spaces (sid, symbol) VALUES (?1, ?2)"
params = ["{{{{steps.sid.json.sid}}}}", "{{{{steps.sid.json.symbol}}}}"]

[[artur.endpoints.steps]]
id = "lookup"
type = "store.query"
store = "artur"
depends_on = ["insert"]
sql = "SELECT sid, symbol FROM spaces WHERE sid = ?1"
params = ["{{{{steps.sid.json.sid}}}}"]

[[artur.tasks]]
name = "sid_create"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"sid\":\"sid-1\",\"symbol\":\"ETH\"}}'"]
stdout_format = "json"
"#,
            db_path.display()
        ),
    )
    .unwrap();

    let cfg = load_config(config_path.to_str().unwrap()).await.unwrap();
    let app = build_router(cfg).await.unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/spaces")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["sid"], "sid-1");
    assert_eq!(value["symbol"], "ETH");
}

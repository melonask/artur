use artur::{config::*, server::build_router};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::ServiceExt;

fn test_config() -> AppConfig {
    AppConfig {
        version: 1,
        server: ServerConfig::default(),
        endpoints: vec![
            EndpointConfig {
                name: "hello".to_string(),
                method: HttpMethod::Get,
                path: "/hello".to_string(),
                action: EndpointAction::RespondStatic,
                process: None,
                response: Some(StaticResponseConfig {
                    status: 200,
                    body: serde_json::json!({ "ok": true, "name": "artur" }),
                    headers: Default::default(),
                }),
            },
            EndpointConfig {
                name: "echo".to_string(),
                method: HttpMethod::Post,
                path: "/echo/{id}".to_string(),
                action: EndpointAction::ProcessRun,
                process: Some("cat_json".to_string()),
                response: None,
            },
            EndpointConfig {
                name: "async".to_string(),
                method: HttpMethod::Post,
                path: "/async".to_string(),
                action: EndpointAction::ProcessRun,
                process: Some("async_json".to_string()),
                response: None,
            },
            EndpointConfig {
                name: "job".to_string(),
                method: HttpMethod::Get,
                path: "/jobs/{job_id}".to_string(),
                action: EndpointAction::JobGet,
                process: None,
                response: None,
            },
        ],
        processes: vec![
            ProcessConfig {
                name: "cat_json".to_string(),
                mode: ProcessMode::Sync,
                command: "sh".to_string(),
                args: vec!["-c".to_string(), "cat".to_string()],
                env: Default::default(),
                working_dir: None,
                timeout_ms: 5000,
                stdin: ProcessStdin::Body,
                stdout_format: ProcessOutputFormat::Json,
            },
            ProcessConfig {
                name: "async_json".to_string(),
                mode: ProcessMode::Async,
                command: "sh".to_string(),
                args: vec!["-c".to_string(), "printf '{\"ok\":true}'".to_string()],
                env: Default::default(),
                working_dir: None,
                timeout_ms: 5000,
                stdin: ProcessStdin::None,
                stdout_format: ProcessOutputFormat::Json,
            },
        ],
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
    assert_eq!(value["process"], "cat_json");
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

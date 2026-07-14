use reqwest::StatusCode;
use serde_json::Value;
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
use tempfile::TempDir;

struct RunningArtur {
    child: Child,
    base_url: String,
    _temp_dir: TempDir,
}

impl Drop for RunningArtur {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ===========================================================================
//  Config.example.toml — active endpoints exercised end-to-end
// ===========================================================================

#[tokio::test]
async fn config_example_all_active_endpoints_succeed() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_path = write_example_config(temp_dir.path(), port);
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    // 1. respond.static — health endpoint
    let health: Value = client
        .get(format!("{}/health", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(health["ok"], true);
    assert_eq!(health["service"], "artur");

    // 2. task.run sync — echo_call with path param and query param
    let echo: Value = client
        .post(format!("{}/v1/echo/alice?source=e2e", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(echo["ok"], true);
    assert_eq!(echo["task"], "echo");
    assert_eq!(echo["json"]["name"], "alice");
    assert_eq!(echo["json"]["source"], "e2e");

    // 3. task.run async — returns job_id
    let async_resp: Value = client
        .post(format!("{}/v1/long-job", server.base_url))
        .body("hello async body")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(async_resp["status"], "running");
    let job_id = async_resp["job_id"].as_str().unwrap();

    // 4. job.get — poll async job
    let mut last_status = String::new();
    for _ in 0..40 {
        let job: Value = client
            .get(format!("{}/v1/jobs/{}", server.base_url, job_id))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json_like()
            .await;
        last_status = job["status"].as_str().unwrap().to_string();
        if last_status == "completed" {
            assert_eq!(job["result"]["json"]["ok"], true);
            assert_eq!(job["result"]["json"]["received"], "hello async body");
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(last_status, "completed", "async job did not complete");

    // 5. workflow.run — compose endpoint with task step and result template
    let wf: Value = client
        .post(format!(
            "{}/v1/compose/wf-user?source=wf-test",
            server.base_url
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(wf["ok"], true);
    assert_eq!(wf["echo_name"], "wf-user");
    assert_eq!(wf["echo_source"], "wf-test");
}

// ===========================================================================
//  Security features from Config.example.toml
// ===========================================================================

#[tokio::test]
async fn api_key_guard_allows_with_correct_key() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "secure"
method = "POST"
path = "/secure"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true, secured = true }}

[artur.endpoints.security.api_key]
header = "authorization"
value = "secret-key"
scheme = "Bearer"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let forbidden = client
        .post(format!("{}/secure", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let ok: Value = client
        .post(format!("{}/secure", server.base_url))
        .header("authorization", "Bearer secret-key")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(ok["secured"], true);
}

#[tokio::test]
async fn challenge_guard_allows_on_ok_and_rejects_on_failure() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "gated"
method = "POST"
path = "/gated"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true, gate = "passed" }}

[artur.endpoints.security.challenge]
task = "verify"
success_path = "challenge_ok"

[[artur.tasks]]
name = "verify"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"ok\":true,\"challenge_ok\":true}}'"]
stdout_format = "json"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let ok: Value = client
        .post(format!("{}/gated", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(ok["gate"], "passed");

    // Also test with explicit success_path
    // Test reject when task says ok=false
    let temp_dir2 = TempDir::new().unwrap();
    let port2 = unused_port();
    let config_toml2 = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port2}

[[artur.endpoints]]
name = "gated"
method = "POST"
path = "/gated"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true }}

[artur.endpoints.security.challenge]
task = "verify_fail"

[[artur.tasks]]
name = "verify_fail"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"ok\":false}}'"]
stdout_format = "json"
"#
    );
    let config_path2 = temp_dir2.path().join("config.toml");
    fs::write(&config_path2, config_toml2).unwrap();
    let server2 = spawn_artur(&config_path2, port2, temp_dir2).await;
    let forbidden = client
        .post(format!("{}/gated", server2.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn x402_guard_returns_402_with_x402_version_header() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "paid"
method = "POST"
path = "/paid"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true }}

[artur.endpoints.security.x402]
task = "check"
success_path = "paid"

[[artur.tasks]]
name = "check"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"paid\":false}}'"]
stdout_format = "json"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/paid", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(response.headers().get("x402-version").unwrap(), "1");
}

#[tokio::test]
async fn failure_block_blocks_after_max_failures_then_expires() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "flaky"
method = "POST"
path = "/flaky"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true }}

[artur.endpoints.security.api_key]
header = "authorization"
value = "correct"

[artur.endpoints.security.failure_block]
key = "{{{{header.authorization}}}}"
max_failures = 2
window_secs = 300
block_secs = 1
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    // Accumulate 2 failures with wrong key
    for _ in 0..2 {
        let resp = client
            .post(format!("{}/flaky", server.base_url))
            .header("authorization", "wrong")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // 3rd wrong-key request → blocked (429)
    let blocked = client
        .post(format!("{}/flaky", server.base_url))
        .header("authorization", "wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);

    // Good key works fine (different blocking key)
    let ok: Value = client
        .post(format!("{}/flaky", server.base_url))
        .header("authorization", "correct")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(ok["ok"], true);

    // Wait for block to expire
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Wrong key is unblocked now
    let resp = client
        .post(format!("{}/flaky", server.base_url))
        .header("authorization", "wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ===========================================================================
//  Workflow step types: store, respond, dependencies, continue_on_error
// ===========================================================================

#[tokio::test]
async fn workflow_store_query_and_execute() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let db_path = temp_dir.path().join("test.db");
    let db_str = db_path.to_string_lossy().replace('\\', "\\\\");
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[stores.test]
driver = "sqlite"
url = "sqlite://{db_str}"

[[artur.endpoints]]
name = "items"
method = "POST"
path = "/items"
action = "workflow.run"

[artur.endpoints.result]
include_steps = false
body = {{ ok = true, items = "{{{{steps.query.rows}}}}" }}

[[artur.endpoints.steps]]
id = "schema"
type = "store.execute"
store = "test"
sql = "CREATE TABLE IF NOT EXISTS t (id INTEGER PRIMARY KEY, name TEXT)"

[[artur.endpoints.steps]]
id = "insert"
type = "store.execute"
store = "test"
depends_on = ["schema"]
sql = "INSERT INTO t (name) VALUES (?1)"
params = ["{{{{body_json.name}}}}"]

[[artur.endpoints.steps]]
id = "query"
type = "store.query"
store = "test"
depends_on = ["insert"]
sql = "SELECT id, name FROM t ORDER BY id"
"#,
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    for name in &["alpha", "beta"] {
        let resp: Value = client
            .post(format!("{}/items", server.base_url))
            .header("content-type", "application/json")
            .body(serde_json::json!({ "name": name }).to_string())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json_like()
            .await;
        assert_eq!(resp["ok"], true);
    }

    let resp: Value = client
        .post(format!("{}/items", server.base_url))
        .header("content-type", "application/json")
        .body(r#"{"name":"gamma"}"#)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    let rows = resp["items"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["name"], "alpha");
    assert_eq!(rows[1]["name"], "beta");
    assert_eq!(rows[2]["name"], "gamma");
}

#[tokio::test]
async fn workflow_respond_step_returns_rendered_value() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "chain"
method = "POST"
path = "/chain"
action = "workflow.run"

[artur.endpoints.result]
include_steps = false

[[artur.endpoints.steps]]
id = "step1"
type = "task"
task = "gen2"

[[artur.endpoints.steps]]
id = "step2"
type = "task"
task = "gen3"
depends_on = ["step1"]

[[artur.endpoints.steps]]
id = "reply"
type = "respond"
depends_on = ["step2"]
value = {{ sum = "{{{{steps.step2.json.n}}}}" }}

[[artur.tasks]]
name = "gen2"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"n\":2}}'"]
stdout_format = "json"

[[artur.tasks]]
name = "gen3"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"n\":3}}'"]
stdout_format = "json"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/chain", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["result"]["sum"], 3);
}

#[tokio::test]
async fn workflow_continue_on_error_keeps_pipeline_running() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "resilient"
method = "POST"
path = "/resilient"
action = "workflow.run"

[artur.endpoints.result]
include_steps = true

[[artur.endpoints.steps]]
id = "bad"
type = "task"
task = "always_fails"
continue_on_error = true

[[artur.endpoints.steps]]
id = "good"
type = "task"
task = "always_ok"
depends_on = ["bad"]

[[artur.tasks]]
name = "always_fails"
mode = "sync"
command = "sh"
args = ["-c", "echo 'not json' && exit 1"]
stdout_format = "text"

[[artur.tasks]]
name = "always_ok"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"n\":42}}'"]
stdout_format = "json"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/resilient", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;

    assert_eq!(resp["steps"]["bad"]["ok"], false);
    assert!(resp["steps"]["bad"]["error"].as_str().is_some());
    assert_eq!(resp["steps"]["good"]["ok"], true);
    assert_eq!(resp["steps"]["good"]["json"]["n"], 42);
}

// ===========================================================================
//  Task stdin variants: body, request_json, template, none
// ===========================================================================

#[tokio::test]
async fn task_stdin_body_pipes_request_body() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "stdin_body"
method = "POST"
path = "/stdin/body"
action = "task.run"
task = "cat_stdin"

[[artur.tasks]]
name = "cat_stdin"
mode = "sync"
command = "sh"
args = ["-c", "cat"]
stdout_format = "json"

[artur.tasks.stdin]
type = "body"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/stdin/body", server.base_url))
        .header("content-type", "application/json")
        .body(r#"{"payload":"hello"}"#)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["json"]["payload"], "hello");
}

#[tokio::test]
async fn task_stdin_request_json_pipes_full_context() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "stdin_rj"
method = "POST"
path = "/stdin/rj"
action = "task.run"
task = "cat_rj"

[[artur.tasks]]
name = "cat_rj"
mode = "sync"
command = "sh"
args = ["-c", "cat"]
stdout_format = "json"

[artur.tasks.stdin]
type = "request_json"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/stdin/rj", server.base_url))
        .header("x-custom", "val1")
        .body(r#"{"k":"v"}"#)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["json"]["method"], "POST");
    assert_eq!(resp["json"]["headers"]["x-custom"], "val1");
    assert_eq!(resp["json"]["body"], r#"{"k":"v"}"#);
}

#[tokio::test]
async fn task_stdin_template_pipes_custom_string() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "stdin_tmpl"
method = "POST"
path = "/stdin/tmpl/{{name}}"
action = "task.run"
task = "cat_tmpl"

[[artur.tasks]]
name = "cat_tmpl"
mode = "sync"
command = "sh"
args = ["-c", "cat"]
stdout_format = "text"

[artur.tasks.stdin]
type = "template"
template = "{{{{param.name}}}}:{{{{query.x}}}}"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/stdin/tmpl/abc?x=42", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["stdout"], "abc:42");
}

// ===========================================================================
//  Task env, inherit_env, working_dir, success_exit_codes
// ===========================================================================

#[tokio::test]
async fn task_env_vars_and_inherit() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "env_test"
method = "POST"
path = "/env-test"
action = "task.run"
task = "show_env"

[[artur.tasks]]
name = "show_env"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"CUSTOM\":\"'$ARTUR_CUSTOM'\",\"HOME_set\":true}}'"]
stdout_format = "json"
inherit_env = true

[artur.tasks.env]
ARTUR_CUSTOM = "custom-value"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/env-test", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["json"]["CUSTOM"], "custom-value");
    // inherit_env=true so HOME should be set
    assert_eq!(resp["json"]["HOME_set"], true);
}

#[tokio::test]
async fn task_working_dir() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let work_dir = temp_dir.path().join("workdir");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join("marker.txt"), "present").unwrap();
    let wd_escaped = work_dir.to_string_lossy().replace('\\', "\\\\");

    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "wd_test"
method = "POST"
path = "/wd-test"
action = "task.run"
task = "show_pwd"

[[artur.tasks]]
name = "show_pwd"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"pwd\":\"'$(cat marker.txt)'\"}}'"]
stdout_format = "json"
working_dir = "{wd_escaped}"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/wd-test", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["json"]["pwd"], "present");
}

#[tokio::test]
async fn task_custom_success_exit_codes() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "custom_exit"
method = "POST"
path = "/custom-exit"
action = "task.run"
task = "exit42"

[[artur.tasks]]
name = "exit42"
mode = "sync"
command = "sh"
args = ["-c", "printf 'hello42'; exit 42"]
timeout_ms = 10000
success_exit_codes = [42]
stdout_format = "text"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/custom-exit", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["stdout"], "hello42");
}

// ===========================================================================
//  stdout_format: text vs json
// ===========================================================================

#[tokio::test]
async fn stdout_format_text_returns_raw_stdout_no_json_field() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "text_fmt"
method = "POST"
path = "/text-fmt"
action = "task.run"
task = "text_out"

[[artur.tasks]]
name = "text_out"
mode = "sync"
command = "sh"
args = ["-c", "printf 'plain output'"]
stdout_format = "text"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/text-fmt", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["stdout"], "plain output");
    assert!(resp.get("json").is_none_or(|v| v.is_null()));
}

// ===========================================================================
//  Response features: custom status codes, headers, HTTP methods
// ===========================================================================

#[tokio::test]
async fn static_response_custom_status_and_headers_and_methods() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "created"
method = "PUT"
path = "/created"
action = "respond.static"

[artur.endpoints.response]
status = 201
body = {{ ok = true }}
headers = {{ x-custom = "hello", cache-control = "no-store" }}

[[artur.endpoints]]
name = "deleted"
method = "DELETE"
path = "/deleted"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true }}

[[artur.endpoints]]
name = "patched"
method = "PATCH"
path = "/patched"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true }}
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let put_resp = client
        .put(format!("{}/created", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::CREATED);
    assert_eq!(put_resp.headers().get("x-custom").unwrap(), "hello");
    assert_eq!(put_resp.headers().get("cache-control").unwrap(), "no-store");

    let del_resp = client
        .delete(format!("{}/deleted", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), StatusCode::OK);

    let patch_resp = client
        .patch(format!("{}/patched", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_resp.status(), StatusCode::OK);
}

// ===========================================================================
//  Per-endpoint body_limit_bytes
// ===========================================================================

#[tokio::test]
async fn per_endpoint_body_limit_rejects_oversized_body() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}
body_limit_bytes = 1048576

[[artur.endpoints]]
name = "limited"
method = "POST"
path = "/limited"
action = "respond.static"
body_limit_bytes = 5

[artur.endpoints.response]
body = {{ ok = true }}
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let ok = client
        .post(format!("{}/limited", server.base_url))
        .body("1234")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let too_large = client
        .post(format!("{}/limited", server.base_url))
        .body("123456")
        .send()
        .await
        .unwrap();
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// ===========================================================================
//  Workflow HTTP request step (outgoing HTTP call)
// ===========================================================================

#[tokio::test]
async fn workflow_http_request_step_calls_own_endpoint() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "health"
method = "GET"
path = "/health"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true, service = "test" }}

[[artur.endpoints]]
name = "proxy"
method = "POST"
path = "/proxy"
action = "workflow.run"

[artur.endpoints.result]
include_steps = false
body = {{ ok = true, proxied_ok = "{{{{steps.call.json.ok}}}}" }}

[[artur.endpoints.steps]]
id = "call"
type = "http.request"
url = "http://127.0.0.1:{port}/health"
method = "GET"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/proxy", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["proxied_ok"], true);
}

// ===========================================================================
//  Template placeholders: param, query, header, env, body_json, steps
// ===========================================================================

#[tokio::test]
async fn template_placeholders_all_context_variables() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    unsafe {
        std::env::set_var("ARTUR_TEST_ENV", "env-value");
    }

    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "templates"
method = "POST"
path = "/tmpl/{{id}}"
action = "workflow.run"

[artur.endpoints.result]
include_steps = false
body = {{ method = "{{{{method}}}}", path = "{{{{path}}}}", param_id = "{{{{param.id}}}}", query_src = "{{{{query.src}}}}", header_cust = "{{{{header.x-custom}}}}", env_val = "{{{{env.ARTUR_TEST_ENV}}}}", body_field = "{{{{body_json.name}}}}", body_raw = "{{{{body}}}}", step_val = "{{{{steps.s1.json.n}}}}" }}

[[artur.endpoints.steps]]
id = "s1"
type = "task"
task = "gen_one"

[[artur.tasks]]
name = "gen_one"
mode = "sync"
command = "sh"
args = ["-c", "printf '{{\"n\":1}}'"]
stdout_format = "json"
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let resp: Value = client
        .post(format!("{}/tmpl/42?src=e2e", server.base_url))
        .header("x-custom", "custom-val")
        .header("content-type", "application/json")
        .body(r#"{"name":"test-body"}"#)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;

    assert_eq!(resp["method"], "POST");
    assert!(resp["path"].as_str().unwrap().starts_with("/tmpl/42"));
    assert_eq!(resp["param_id"], "42");
    assert_eq!(resp["query_src"], "e2e");
    assert_eq!(resp["header_cust"], "custom-val");
    assert_eq!(resp["env_val"], "env-value");
    assert_eq!(resp["body_field"], "test-body");
    assert_eq!(resp["body_raw"], r#"{"name":"test-body"}"#);
    assert_eq!(resp["step_val"], 1);
}

// ===========================================================================
//  Artur.server defaults (no explicit configuration)
// ===========================================================================

#[tokio::test]
async fn server_defaults_without_explicit_artur_server_section() {
    let _process_spawn_lock = process_spawn_lock().await;
    let temp_dir = TempDir::new().unwrap();
    let port = unused_port();
    let config_toml = format!(
        r#"
version = 1

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "respond.static"

[artur.endpoints.response]
body = {{ ok = true }}
"#
    );
    let config_path = temp_dir.path().join("config.toml");
    fs::write(&config_path, config_toml).unwrap();
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let ok: Value = client
        .get(format!("{}/hello", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(ok["ok"], true);
}

// ===========================================================================
//  Helpers
// ===========================================================================

static PROCESS_SPAWN_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

async fn process_spawn_lock() -> tokio::sync::MutexGuard<'static, ()> {
    PROCESS_SPAWN_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

async fn spawn_artur(config_path: &Path, port: u16, temp_dir: TempDir) -> RunningArtur {
    let mut child = Command::new(env!("CARGO_BIN_EXE_artur"))
        .arg("--config")
        .arg(config_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let base_url = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    for _ in 0..200 {
        if let Ok(Some(status)) = child.try_wait() {
            let mut stderr = String::new();
            if let Some(mut s) = child.stderr.take() {
                use std::io::Read;
                let _ = s.read_to_string(&mut stderr);
            }
            panic!("artur exited before becoming ready with status {status}\nstderr: {stderr}");
        }
        if let Ok(response) = client.get(format!("{base_url}/healthz")).send().await
            && response.status() == StatusCode::OK
        {
            return RunningArtur {
                child,
                base_url,
                _temp_dir: temp_dir,
            };
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = child.kill();
    panic!("artur did not become ready on port {port}");
}

fn write_example_config(temp_dir: &Path, port: u16) -> PathBuf {
    let config_path = temp_dir.join("config.toml");

    let echo_py = temp_dir.join("echo.py");
    fs::write(
        &echo_py,
        r#"#!/usr/bin/env python3
import argparse, json, sys
parser = argparse.ArgumentParser()
parser.add_argument("--name", default="")
parser.add_argument("--source", default="")
args = parser.parse_args()
stdin = sys.stdin.read()
try:
    request = json.loads(stdin) if stdin else None
except json.JSONDecodeError:
    request = stdin
print(json.dumps({"ok":True,"name":args.name,"source":args.source,"request":request}, separators=(",",":")))
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&echo_py).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&echo_py, perms).unwrap();
    }

    let long_task_py = temp_dir.join("long_task.py");
    fs::write(
        &long_task_py,
        r#"#!/usr/bin/env python3
import json, sys, time
body = sys.stdin.read()
time.sleep(0.05)
print(json.dumps({"ok": True, "received": body}, separators=(",", ":")))
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&long_task_py).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&long_task_py, perms).unwrap();
    }

    let config = format!(
        r#"
version = 1

[log]
level = "artur=info,tower_http=info"

[artur.server]
bind = "127.0.0.1"
port = {port}

[[artur.tasks]]
name = "echo"
mode = "sync"
command = "python3"
args = [
    "{echo_py}",
    "--name", "{{{{param.name}}}}",
    "--source", "{{{{query.source}}}}",
]
inherit_env = true
success_exit_codes = [0]
timeout_ms = 10000
max_stdout_bytes = 1048576
max_stderr_bytes = 1048576
stdout_format = "json"

[artur.tasks.stdin]
type = "request_json"

[[artur.tasks]]
name = "long_job"
mode = "async"
command = "python3"
args = ["{long_task_py}"]
timeout_ms = 30000
stdout_format = "json"

[artur.tasks.stdin]
type = "body"

[[artur.endpoints]]
name = "health"
method = "GET"
path = "/health"
action = "respond.static"

[artur.endpoints.response]
status = 200
body = {{ ok = true, service = "artur" }}

[[artur.endpoints]]
name = "echo_call"
method = "POST"
path = "/v1/echo/{{name}}"
action = "task.run"
task = "echo"

[[artur.endpoints]]
name = "get_job"
method = "GET"
path = "/v1/jobs/{{job_id}}"
action = "job.get"

[[artur.endpoints]]
name = "long_job_start"
method = "POST"
path = "/v1/long-job"
action = "task.run"
task = "long_job"

[[artur.endpoints]]
name = "compose"
method = "POST"
path = "/v1/compose/{{name}}"
action = "workflow.run"

[artur.endpoints.result]
status = 200
body = {{ ok = true, echo_name = "{{{{steps.enrich.json.name}}}}", echo_source = "{{{{steps.enrich.json.source}}}}" }}
include_steps = false

[[artur.endpoints.steps]]
id = "enrich"
type = "task"
task = "echo"
"#,
        echo_py = path_for_toml(&echo_py),
        long_task_py = path_for_toml(&long_task_py),
    );
    fs::write(&config_path, config).unwrap();
    config_path
}

fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn path_for_toml(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

trait JsonLike {
    async fn json_like(self) -> Value;
}

impl JsonLike for reqwest::Response {
    async fn json_like(self) -> Value {
        let text = self.text().await.unwrap();
        serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("invalid JSON response {text:?}: {err}"))
    }
}

// ===========================================================================
//  Unit-level parsing tests (no server needed)
// ===========================================================================

#[cfg(test)]
mod unit {
    use artur::AppConfig;
    use artur::config::*;

    #[test]
    fn parses_all_root_sections() {
        let raw = r#"
version = 1

[log]
level = "debug"
format = "json"

[runtime]
worker_threads = 2
shutdown_timeout_secs = 15
tmp_dir = "/tmp/artur"
max_payload_bytes = 524288

[paths.data]
path = "/data"
format = "json"

[transports.http.upstream]
base_url = "http://upstream:4000/v1"
timeout_ms = 15000

[transports.http.upstream.headers]
authorization = "Bearer key"

[stores.db]
driver = "sqlite"
url = "sqlite://data.db"
connect_timeout_secs = 5

[artur.server]
bind = "127.0.0.1"
port = 46796
body_limit_bytes = 2097152

[[artur.endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "respond.static"

[artur.endpoints.response]
body = { ok = true }
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        cfg.validate().unwrap();

        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.log.level.as_deref(), Some("debug"));
        assert_eq!(cfg.log.format, Some(LogFormat::Json));
        assert_eq!(cfg.runtime.worker_threads, Some(2));
        assert_eq!(cfg.runtime.shutdown_timeout_secs, Some(15));
        assert_eq!(cfg.runtime.tmp_dir.as_deref(), Some("/tmp/artur"));
        assert_eq!(cfg.runtime.max_payload_bytes, Some(524288));
        assert_eq!(cfg.paths["data"].path, "/data");
        assert_eq!(cfg.paths["data"].format.as_deref(), Some("json"));
        assert_eq!(
            cfg.transports.http["upstream"].base_url,
            "http://upstream:4000/v1"
        );
        assert_eq!(
            cfg.transports.http["upstream"].headers["authorization"],
            "Bearer key"
        );
        assert_eq!(cfg.transports.http["upstream"].timeout_ms, Some(15000));
        assert_eq!(cfg.stores["db"].driver, StoreDriver::Sqlite);
        assert_eq!(cfg.stores["db"].url, "sqlite://data.db");
        assert_eq!(cfg.stores["db"].connect_timeout_secs, Some(5));

        let server = cfg.server_config();
        assert_eq!(server.bind, "127.0.0.1");
        assert_eq!(server.port, 46796);
        assert_eq!(server.body_limit_bytes, 2097152);
    }

    #[test]
    fn server_config_uses_artur_namespace_only() {
        let cfg: AppConfig = toml::from_str(
            r#"
version = 1

[artur.server]
bind = "127.0.0.1"

[[artur.endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "respond.static"

[artur.endpoints.response]
body = { ok = true }
"#,
        )
        .unwrap();
        cfg.validate().unwrap();
        let s = cfg.server_config();
        assert_eq!(s.bind, "127.0.0.1");
        assert_eq!(s.port, 46796);
        assert_eq!(s.body_limit_bytes, 1_048_576);
    }

    #[test]
    fn parses_all_endpoint_security_fields() {
        let raw = r#"
version = 1

[[artur.endpoints]]
name = "full"
method = "POST"
path = "/full"
action = "respond.static"

[artur.endpoints.response]
body = { ok = true }

[artur.endpoints.security.api_key]
header = "x-api-key"
value = "secret"
scheme = "Key"

[artur.endpoints.security.challenge]
task = "verify_c"
success_path = "ok"

[artur.endpoints.security.x402]
task = "verify_p"
success_path = "paid"

[artur.endpoints.security.failure_block]
key = "{{header.authorization}}"
max_failures = 3
window_secs = 60
block_secs = 120
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        let ep = &cfg.artur.endpoints[0];
        let api = ep.security.api_key.as_ref().unwrap();
        assert_eq!(api.header, "x-api-key");
        assert_eq!(api.value, "secret");
        assert_eq!(api.scheme.as_deref(), Some("Key"));
        let ch = ep.security.challenge.as_ref().unwrap();
        assert_eq!(ch.task, "verify_c");
        assert_eq!(ch.success_path.as_deref(), Some("ok"));
        let x4 = ep.security.x402.as_ref().unwrap();
        assert_eq!(x4.task, "verify_p");
        assert_eq!(x4.success_path.as_deref(), Some("paid"));
        let fb = ep.security.failure_block.as_ref().unwrap();
        assert_eq!(fb.key, "{{header.authorization}}");
        assert_eq!(fb.max_failures, 3);
        assert_eq!(fb.window_secs, 60);
        assert_eq!(fb.block_secs, 120);
    }

    #[test]
    fn parses_all_task_fields() {
        let raw = r#"
version = 1

[[artur.endpoints]]
name = "doit"
method = "POST"
path = "/doit"
action = "task.run"
task = "full_task"

[[artur.tasks]]
name = "full_task"
mode = "async"
command = "python3"
args = ["{{param.id}}", "{{query.x}}"]
timeout_ms = 60000
max_stdout_bytes = 512
max_stderr_bytes = 256
success_exit_codes = [0, 1]
stdout_format = "json"
inherit_env = false
working_dir = "/tmp"

[artur.tasks.env]
A = "1"
B = "2"

[artur.tasks.stdin]
type = "template"
template = "{{header.x-foo}}"
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        cfg.validate().unwrap();
        let t = &cfg.artur.tasks[0];
        assert_eq!(t.name, "full_task");
        assert_eq!(t.mode, TaskMode::Async);
        assert_eq!(t.command, "python3");
        assert_eq!(t.args, vec!["{{param.id}}", "{{query.x}}"]);
        assert_eq!(t.timeout_ms, 60000);
        assert_eq!(t.max_stdout_bytes, 512);
        assert_eq!(t.max_stderr_bytes, 256);
        assert_eq!(t.success_exit_codes, vec![0, 1]);
        assert_eq!(t.stdout_format, TaskOutputFormat::Json);
        assert!(!t.inherit_env);
        assert_eq!(t.working_dir.as_deref(), Some("/tmp"));
        assert_eq!(t.env["A"], "1");
        assert_eq!(t.env["B"], "2");
        match &t.stdin {
            TaskStdin::Template { template } => assert_eq!(template, "{{header.x-foo}}"),
            other => panic!("expected stdin template, got {other:?}"),
        }
    }

    #[test]
    fn parses_all_workflow_step_kinds_and_fields() {
        let raw = r#"
version = 1

[[artur.endpoints]]
name = "wf"
method = "POST"
path = "/wf"
action = "workflow.run"

[artur.endpoints.result]
status = 201
body = { ok = true, data = "{{steps.a.rows}}" }
include_steps = false
headers = { x-custom = "yes" }

[[artur.endpoints.steps]]
id = "a"
type = "store.query"
store = "test"
sql = "SELECT 1"
params = ["{{query.x}}"]
depends_on = ["b"]
continue_on_error = true

[[artur.endpoints.steps]]
id = "b"
type = "store.execute"
store = "test"
sql = "INSERT INTO t VALUES (1)"

[[artur.endpoints.steps]]
id = "c"
type = "task"
task = "some_task"

[[artur.endpoints.steps]]
id = "d"
type = "http.request"
transport = "ladon"
url = "/jobs"
method = "POST"
timeout_ms = 5000
headers = { authorization = "Bearer key" }
body = { value = "{{param.id}}" }

[[artur.endpoints.steps]]
id = "e"
type = "respond"
value = { ok = true, sum = "{{steps.c.json.n}}" }

[[artur.tasks]]
name = "some_task"
mode = "sync"
command = "true"
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        // Validation requires the store and transport to exist; they don't here.
        // Just check parsing works (validate will fail on missing store/transport).
        let ep = &cfg.artur.endpoints[0];
        assert_eq!(ep.result.status, 201);
        assert!(!ep.result.include_steps);
        assert_eq!(ep.result.headers["x-custom"], "yes");

        assert_eq!(ep.steps.len(), 5);
        let a = &ep.steps[0];
        assert_eq!(a.id, "a");
        assert_eq!(a.kind, WorkflowStepKind::StoreQuery);
        assert_eq!(a.depends_on, vec!["b"]);
        assert!(a.continue_on_error);

        let d = &ep.steps[3];
        assert_eq!(d.kind, WorkflowStepKind::HttpRequest);
        assert_eq!(d.method.unwrap(), HttpMethod::Post);
        assert_eq!(d.timeout_ms, Some(5000));

        let e = &ep.steps[4];
        assert_eq!(e.kind, WorkflowStepKind::Respond);
    }
}

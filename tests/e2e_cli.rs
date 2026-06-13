use reqwest::StatusCode;
use serde_json::Value;
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
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

#[tokio::test]
async fn cli_serves_js_npx_and_rust_processes_end_to_end() {
    require_command("node");
    require_command("npx");
    require_command("rustc");

    let temp_dir = TempDir::new().unwrap();
    let rust_helper = compile_rust_helper(temp_dir.path());
    let npx_dir = create_local_npx_helper(temp_dir.path());
    let port = unused_port();
    let config_path = write_e2e_config(temp_dir.path(), port, &rust_helper, &npx_dir);
    let server = spawn_artur(&config_path, port, temp_dir).await;
    let client = reqwest::Client::new();

    let hello: Value = client
        .get(format!("{}/v1/hello", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(hello["ok"], true);

    let js: Value = client
        .post(format!(
            "{}/v1/process/js/alice?source=e2e",
            server.base_url
        ))
        .header("content-type", "application/json")
        .body(r#"{"message":"hello from js"}"#)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(js["ok"], true);
    assert_eq!(js["json"]["runtime"], "node");
    assert_eq!(js["json"]["name"], "alice");
    assert_eq!(js["json"]["source"], "e2e");
    assert_eq!(js["json"]["body"]["message"], "hello from js");

    let npx: Value = client
        .post(format!("{}/v1/process/npx?value=package", server.base_url))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(npx["ok"], true);
    assert_eq!(npx["json"]["runtime"], "npx");
    assert_eq!(npx["json"]["value"], "package");

    let rust: Value = client
        .post(format!("{}/v1/process/rust/42", server.base_url))
        .header("content-type", "application/json")
        .body(r#"{"language":"rust"}"#)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json_like()
        .await;
    assert_eq!(rust["ok"], true);
    assert_eq!(rust["json"]["runtime"], "rust");
    assert_eq!(rust["json"]["id"], "42");
    assert_eq!(rust["json"]["stdin"], r#"{"language":"rust"}"#);
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
    for _ in 0..100 {
        if let Ok(Some(status)) = child.try_wait() {
            panic!("artur exited before becoming ready with status {status}");
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

fn write_e2e_config(temp_dir: &Path, port: u16, rust_helper: &Path, npx_dir: &Path) -> PathBuf {
    let config_path = temp_dir.join("e2e.toml");
    let js = r#"
const fs = require('fs');
const request = JSON.parse(fs.readFileSync(0, 'utf8'));
console.log(JSON.stringify({runtime:'node', name:process.argv[1], source:request.query.source, body:request.body_json}));
"#;
    let config = format!(
        r#"
version = 1

[server]
bind = "127.0.0.1"
port = {port}
body_limit_bytes = 1048576

[[endpoints]]
name = "hello"
method = "GET"
path = "/v1/hello"
action = "respond.static"

[endpoints.response]
status = 200
body = {{ ok = true, service = "artur-e2e" }}

[[endpoints]]
name = "js"
method = "POST"
path = "/v1/process/js/{{name}}"
action = "process.run"
process = "js_helper"

[[processes]]
name = "js_helper"
mode = "sync"
command = "node"
args = ["-e", "{js}", "{{{{param.name}}}}"]
timeout_ms = 10000
stdout_format = "json"

[processes.stdin]
type = "request_json"

[[endpoints]]
name = "npx"
method = "POST"
path = "/v1/process/npx"
action = "process.run"
process = "npx_helper"

[[processes]]
name = "npx_helper"
mode = "sync"
command = "npx"
args = ["--no-install", "artur-npx-helper", "{{{{query.value}}}}"]
working_dir = "{npx_dir}"
timeout_ms = 10000
stdout_format = "json"

[[endpoints]]
name = "rust"
method = "POST"
path = "/v1/process/rust/{{id}}"
action = "process.run"
process = "rust_helper"

[[processes]]
name = "rust_helper"
mode = "sync"
command = "{rust_helper}"
args = ["{{{{param.id}}}}"]
timeout_ms = 10000
stdout_format = "json"

[processes.stdin]
type = "body"
"#,
        js = toml_escape(js),
        npx_dir = path_for_toml(npx_dir),
        rust_helper = path_for_toml(rust_helper),
    );
    fs::write(&config_path, config).unwrap();
    config_path
}

fn create_local_npx_helper(temp_dir: &Path) -> PathBuf {
    let package_dir = temp_dir.join("npx-package");
    let bin_dir = package_dir.join("node_modules/.bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(package_dir.join("package.json"), r#"{"private":true}"#).unwrap();
    let script = bin_dir.join("artur-npx-helper");
    fs::write(
        &script,
        "#!/usr/bin/env node\nconsole.log(JSON.stringify({runtime:'npx', value:process.argv[2]}));\n",
    )
    .unwrap();
    make_executable(&script);
    package_dir
}

fn compile_rust_helper(temp_dir: &Path) -> PathBuf {
    let source = temp_dir.join("rust_helper.rs");
    let binary = temp_dir.join(format!("rust-helper{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        &source,
        r#"
use std::io::{self, Read};

fn main() {
    let id = std::env::args().nth(1).unwrap_or_default();
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin).unwrap();
    let escaped = stdin.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    println!("{{\"runtime\":\"rust\",\"id\":\"{}\",\"stdin\":\"{}\"}}", id, escaped);
}
"#,
    )
    .unwrap();
    let status = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile Rust e2e helper");
    binary
}

fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn require_command(command: &str) {
    let status = Command::new(command)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|err| panic!("{command} is required for e2e tests: {err}"));
    assert!(status.success(), "{command} --version failed");
}

fn toml_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn path_for_toml(path: &Path) -> String {
    toml_escape(&path.to_string_lossy())
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
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

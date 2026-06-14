use crate::{
    config::{TaskConfig, TaskMode, TaskOutputFormat, TaskStdin},
    error::{ArturError, Result},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, process::Command, sync::RwLock, time::timeout};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    pub method: String,
    pub uri: String,
    pub path: String,
    pub params: BTreeMap<String, String>,
    pub query: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub body_json: Option<Value>,
    pub steps: BTreeMap<String, Value>,
}

impl RequestContext {
    pub fn from_parts(
        method: String,
        uri: String,
        path: String,
        params: BTreeMap<String, String>,
        query: BTreeMap<String, String>,
        headers: BTreeMap<String, String>,
        body: Bytes,
    ) -> Self {
        let body = String::from_utf8_lossy(&body).to_string();
        let body_json = serde_json::from_str(&body).ok();
        Self {
            method,
            uri,
            path,
            params,
            query,
            headers,
            body,
            body_json,
            steps: BTreeMap::new(),
        }
    }

    pub fn request_json(&self) -> Value {
        serde_json::json!({
            "method": self.method.clone(),
            "uri": self.uri.clone(),
            "path": self.path.clone(),
            "params": self.params.clone(),
            "query": self.query.clone(),
            "headers": self.headers.clone(),
            "body": self.body.clone(),
            "body_json": self.body_json.clone(),
            "steps": self.steps.clone(),
        })
    }

    pub fn with_steps(&self, steps: BTreeMap<String, Value>) -> Self {
        let mut cloned = self.clone();
        cloned.steps = steps;
        cloned
    }
}

#[derive(Debug, Clone, Default)]
pub struct JobStore {
    jobs: Arc<RwLock<HashMap<String, JobRecord>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobRecord {
    pub id: String,
    pub status: JobStatus,
    pub task: String,
    pub result: Option<TaskOutput>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskOutput {
    pub ok: bool,
    pub task: String,
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_parse_error: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub stdout_truncated: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub stderr_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum TaskRunResponse {
    Immediate(TaskOutput),
    Accepted { job_id: String, status: JobStatus },
}

impl JobStore {
    pub async fn get(&self, id: &str) -> Option<JobRecord> {
        self.jobs.read().await.get(id).cloned()
    }

    async fn insert_running(&self, id: String, task: String) {
        self.jobs.write().await.insert(
            id.clone(),
            JobRecord {
                id,
                status: JobStatus::Running,
                task,
                result: None,
            },
        );
    }

    async fn finish(&self, id: &str, result: Result<TaskOutput>) {
        let mut jobs = self.jobs.write().await;
        if let Some(record) = jobs.get_mut(id) {
            match result {
                Ok(output) => {
                    record.status = if output.ok {
                        JobStatus::Completed
                    } else {
                        JobStatus::Failed
                    };
                    record.result = Some(output);
                }
                Err(err) => {
                    record.status = JobStatus::Failed;
                    record.result = Some(TaskOutput {
                        ok: false,
                        task: record.task.clone(),
                        status_code: None,
                        stdout: String::new(),
                        stderr: err.to_string(),
                        timed_out: false,
                        duration_ms: 0,
                        json_parse_error: None,
                        stdout_truncated: false,
                        stderr_truncated: false,
                        json: None,
                    });
                }
            }
        }
    }
}

pub async fn run_or_enqueue(
    cfg: TaskConfig,
    request: RequestContext,
    jobs: JobStore,
) -> Result<TaskRunResponse> {
    match cfg.mode {
        TaskMode::Sync => Ok(TaskRunResponse::Immediate(run_task(&cfg, &request).await?)),
        TaskMode::Async => {
            let job_id = Uuid::new_v4().to_string();
            jobs.insert_running(job_id.clone(), cfg.name.clone()).await;
            let jobs_for_task = jobs.clone();
            let cfg_for_task = cfg.clone();
            let request_for_task = request.clone();
            let job_id_for_task = job_id.clone();
            tokio::spawn(async move {
                let result = run_task(&cfg_for_task, &request_for_task).await;
                jobs_for_task.finish(&job_id_for_task, result).await;
            });
            Ok(TaskRunResponse::Accepted {
                job_id,
                status: JobStatus::Running,
            })
        }
    }
}

pub async fn run_task(cfg: &TaskConfig, request: &RequestContext) -> Result<TaskOutput> {
    let started = Instant::now();
    let args: Vec<String> = cfg
        .args
        .iter()
        .map(|arg| render_template(arg, request))
        .collect::<Result<Vec<_>>>()?;

    let mut command = Command::new(&cfg.command);
    command.kill_on_drop(true);
    if !cfg.inherit_env {
        command.env_clear();
    }
    command.args(args);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    if let Some(working_dir) = &cfg.working_dir {
        command.current_dir(render_template(working_dir, request)?);
    }
    for (key, value) in &cfg.env {
        command.env(key, render_template(value, request)?);
    }

    let stdin_payload = render_stdin(&cfg.stdin, request)?;
    let output_result = if let Some(stdin_payload) = stdin_payload {
        command.stdin(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|err| ArturError::Process(format!("failed to spawn {}: {err}", cfg.name)))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(stdin_payload.as_bytes()).await?;
        }
        timeout(
            Duration::from_millis(cfg.timeout_ms),
            child.wait_with_output(),
        )
        .await
    } else {
        timeout(Duration::from_millis(cfg.timeout_ms), command.output()).await
    };

    match output_result {
        Ok(Ok(output)) => {
            let (stdout, stdout_truncated) =
                bytes_to_limited_string(&output.stdout, cfg.max_stdout_bytes);
            let (stderr, stderr_truncated) =
                bytes_to_limited_string(&output.stderr, cfg.max_stderr_bytes);
            let mut json_parse_error = None;
            let json = match cfg.stdout_format {
                TaskOutputFormat::Text => None,
                TaskOutputFormat::Json => match serde_json::from_str(&stdout) {
                    Ok(value) => Some(value),
                    Err(err) => {
                        json_parse_error = Some(err.to_string());
                        None
                    }
                },
            };
            let status_code = output.status.code();
            let exit_ok = status_code
                .map(|code| cfg.success_exit_codes.contains(&code))
                .unwrap_or(false);
            let json_ok = cfg.stdout_format != TaskOutputFormat::Json || json_parse_error.is_none();
            Ok(TaskOutput {
                ok: exit_ok && json_ok,
                task: cfg.name.clone(),
                status_code,
                stdout,
                stderr,
                timed_out: false,
                duration_ms: started.elapsed().as_millis(),
                json_parse_error,
                stdout_truncated,
                stderr_truncated,
                json,
            })
        }
        Ok(Err(err)) => Err(ArturError::Process(format!(
            "failed to run process {}: {err}",
            cfg.name
        ))),
        Err(_) => Ok(TaskOutput {
            ok: false,
            task: cfg.name.clone(),
            status_code: None,
            stdout: String::new(),
            stderr: format!("process timed out after {} ms", cfg.timeout_ms),
            timed_out: true,
            duration_ms: started.elapsed().as_millis(),
            json_parse_error: None,
            stdout_truncated: false,
            stderr_truncated: false,
            json: None,
        }),
    }
}

fn bytes_to_limited_string(bytes: &[u8], limit: usize) -> (String, bool) {
    let truncated = bytes.len() > limit;
    let visible = if truncated { &bytes[..limit] } else { bytes };
    (String::from_utf8_lossy(visible).to_string(), truncated)
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn render_stdin(cfg: &TaskStdin, request: &RequestContext) -> Result<Option<String>> {
    match cfg {
        TaskStdin::None => Ok(None),
        TaskStdin::Body => Ok(Some(request.body.clone())),
        TaskStdin::RequestJson => Ok(Some(serde_json::to_string(&request.request_json())?)),
        TaskStdin::Template { template } => Ok(Some(render_template(template, request)?)),
    }
}

pub fn render_template(template: &str, request: &RequestContext) -> Result<String> {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            return Err(ArturError::Config(format!(
                "unclosed template expression in {template:?}"
            )));
        };
        let key = after_start[..end].trim();
        rendered.push_str(&lookup_template_value(key, request));
        rest = &after_start[end + 2..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

pub fn lookup_template_value(key: &str, request: &RequestContext) -> String {
    lookup_template_json_value(key, request)
        .map(|value| json_scalar_to_string(&value))
        .unwrap_or_default()
}

pub fn lookup_template_json_value(key: &str, request: &RequestContext) -> Option<Value> {
    match key {
        "method" => Some(Value::String(request.method.clone())),
        "uri" => Some(Value::String(request.uri.clone())),
        "path" => Some(Value::String(request.path.clone())),
        "body" => Some(Value::String(request.body.clone())),
        "request" | "request_json" => Some(request.request_json()),
        "body_json" => request.body_json.clone(),
        "steps" => Some(Value::Object(
            request
                .steps
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Map<String, Value>>(),
        )),
        _ if key.starts_with("param.") => {
            Some(Value::String(lookup_map(&request.params, &key[6..])))
        }
        _ if key.starts_with("query.") => {
            Some(Value::String(lookup_map(&request.query, &key[6..])))
        }
        _ if key.starts_with("header.") => Some(Value::String(lookup_map(
            &request.headers,
            &key[7..].to_ascii_lowercase(),
        ))),
        _ if key.starts_with("env.") => std::env::var(&key[4..]).ok().map(Value::String),
        _ if key.starts_with("body_json.") => {
            lookup_json_path_value(request.body_json.as_ref(), &key[10..])
        }
        _ if key.starts_with("steps.") => {
            let steps = Value::Object(
                request
                    .steps
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Map<String, Value>>(),
            );
            lookup_json_path_value(Some(&steps), &key[6..])
        }
        _ if key.starts_with("step.") => {
            let steps = Value::Object(
                request
                    .steps
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Map<String, Value>>(),
            );
            lookup_json_path_value(Some(&steps), &key[5..])
        }
        _ => None,
    }
}

fn lookup_map(map: &BTreeMap<String, String>, key: &str) -> String {
    map.get(key).cloned().unwrap_or_default()
}

fn lookup_json_path_value(value: Option<&Value>, path: &str) -> Option<Value> {
    let mut value = value?;
    if path.trim().is_empty() {
        return Some(value.clone());
    }
    for part in path.split('.') {
        match value {
            Value::Object(map) => {
                value = map.get(part)?;
            }
            Value::Array(items) => {
                value = items.get(part.parse::<usize>().ok()?)?;
            }
            _ => return None,
        }
    }
    Some(value.clone())
}

pub fn render_json_value(value: &Value, request: &RequestContext) -> Result<Value> {
    match value {
        Value::String(template) => {
            if let Some(key) = whole_template_key(template)
                && let Some(value) = lookup_template_json_value(key, request)
            {
                return Ok(value);
            }
            Ok(Value::String(render_template(template, request)?))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| render_json_value(item, request))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| Ok((key.clone(), render_json_value(value, request)?)))
            .collect::<Result<Map<String, Value>>>()
            .map(Value::Object),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
    }
}

fn whole_template_key(template: &str) -> Option<&str> {
    let trimmed = template.trim();
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
        let key = trimmed[2..trimmed.len() - 2].trim();
        if !key.contains("{{") && !key.contains("}}") {
            return Some(key);
        }
    }
    None
}

fn json_scalar_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

pub fn header_map_to_btree(headers: &axum::http::HeaderMap) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(value) = value.to_str() {
            out.insert(name.as_str().to_ascii_lowercase(), value.to_string());
        }
    }
    out
}

pub fn hashmap_to_btree(input: HashMap<String, String>) -> BTreeMap<String, String> {
    input.into_iter().collect()
}

pub fn btree_to_json_object(input: &BTreeMap<String, String>) -> Value {
    Value::Object(
        input
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect::<Map<String, Value>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RequestContext {
        RequestContext {
            method: "POST".to_string(),
            uri: "/run/abc?x=1".to_string(),
            path: "/run/abc".to_string(),
            params: BTreeMap::from([("id".to_string(), "abc".to_string())]),
            query: BTreeMap::from([("x".to_string(), "1".to_string())]),
            headers: BTreeMap::from([("authorization".to_string(), "Bearer token".to_string())]),
            body: r#"{"name":"Ada","items":["a","b"]}"#.to_string(),
            body_json: Some(serde_json::json!({ "name": "Ada", "items": ["a", "b"] })),
            steps: BTreeMap::from([(
                "sid".to_string(),
                serde_json::json!({ "json": { "sid": "123" } }),
            )]),
        }
    }

    #[test]
    fn renders_templates_from_request_context() {
        let rendered = render_template(
            "{{method}} {{param.id}} {{query.x}} {{header.authorization}} {{body_json.name}} {{body_json.items.1}}",
            &context(),
        )
        .unwrap();
        assert_eq!(rendered, "POST abc 1 Bearer token Ada b");
    }

    #[test]
    fn leaves_unknown_template_as_empty() {
        assert_eq!(render_template("x{{missing}}y", &context()).unwrap(), "xy");
    }

    #[test]
    fn renders_json_values_without_stringifying_whole_templates() {
        let rendered = render_json_value(
            &serde_json::json!({
                "sid": "{{steps.sid.json.sid}}",
                "request": "{{request_json}}"
            }),
            &context(),
        )
        .unwrap();
        assert_eq!(rendered["sid"], "123");
        assert_eq!(rendered["request"]["method"], "POST");
    }
}

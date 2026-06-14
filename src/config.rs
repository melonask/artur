use crate::error::{ArturError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    /// Shared configuration schema version. Current schema is version = 1.
    pub version: u32,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub http: HttpConfig,
    #[serde(default)]
    pub stores: BTreeMap<String, StoreConfig>,
    #[serde(default)]
    pub paths: BTreeMap<String, PathConfig>,
    #[serde(default)]
    pub transports: TransportsConfig,
    #[serde(default)]
    pub artur: ArturConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            log: LogConfig::default(),
            runtime: RuntimeConfig::default(),
            http: HttpConfig::default(),
            stores: BTreeMap::new(),
            paths: BTreeMap::new(),
            transports: TransportsConfig::default(),
            artur: ArturConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArturConfig {
    #[serde(default)]
    pub server: ArturServerConfig,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    #[serde(default)]
    pub tasks: Vec<TaskConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArturServerConfig {
    #[serde(default)]
    pub bind: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub body_limit_bytes: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub body_limit_bytes: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LogConfig {
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub worker_threads: Option<usize>,
    #[serde(default)]
    pub shutdown_timeout_secs: Option<u64>,
    #[serde(default)]
    pub tmp_dir: Option<String>,
    #[serde(default)]
    pub max_payload_bytes: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct HttpConfig {
    #[serde(default)]
    pub bind: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub max_body_bytes: Option<usize>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PathConfig {
    pub path: String,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TransportsConfig {
    #[serde(default)]
    pub http: BTreeMap<String, HttpTransportConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpTransportConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StoreConfig {
    pub driver: StoreDriver,
    pub url: String,
    #[serde(default)]
    pub migrate: bool,
    #[serde(default)]
    pub connect_timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_connections: Option<u32>,
}

#[derive(Debug, Copy, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StoreDriver {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub name: String,
    pub method: HttpMethod,
    pub path: String,
    pub action: EndpointAction,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub response: Option<StaticResponseConfig>,
    #[serde(default)]
    pub security: EndpointSecurityConfig,
    #[serde(default)]
    pub body_limit_bytes: Option<usize>,
    #[serde(default)]
    pub steps: Vec<WorkflowStepConfig>,
    #[serde(default)]
    pub result: WorkflowResponseConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StaticResponseConfig {
    #[serde(default = "default_static_status")]
    pub status: u16,
    #[serde(default = "default_static_body")]
    pub body: Value,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowStepConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: WorkflowStepKind,
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// `type = "task"`: task id from `[[artur.tasks]]`.
    #[serde(default)]
    pub task: Option<String>,

    /// `type = "store.query"` or `type = "store.execute"`.
    #[serde(default)]
    pub store: Option<String>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub params: Vec<String>,

    /// `type = "http.request"`.
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub method: Option<HttpMethod>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Value,
    #[serde(default)]
    pub timeout_ms: Option<u64>,

    /// `type = "respond"`.
    #[serde(default = "default_workflow_value")]
    pub value: Value,

    #[serde(default)]
    pub continue_on_error: bool,
}

#[derive(Debug, Copy, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepKind {
    Task,
    #[serde(rename = "store.query")]
    StoreQuery,
    #[serde(rename = "store.execute")]
    StoreExecute,
    #[serde(rename = "http.request")]
    HttpRequest,
    Respond,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowResponseConfig {
    #[serde(default = "default_static_status")]
    pub status: u16,
    #[serde(default = "default_workflow_body")]
    pub body: Value,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_workflow_include_steps")]
    pub include_steps: bool,
}

impl Default for WorkflowResponseConfig {
    fn default() -> Self {
        Self {
            status: default_static_status(),
            body: default_workflow_body(),
            headers: BTreeMap::new(),
            include_steps: default_workflow_include_steps(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointSecurityConfig {
    #[serde(default)]
    pub api_key: Option<ApiKeySecurityConfig>,
    #[serde(default)]
    pub challenge: Option<SecurityTaskConfig>,
    #[serde(default)]
    pub x402: Option<SecurityTaskConfig>,
    #[serde(default)]
    pub failure_block: Option<FailureBlockConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeySecurityConfig {
    #[serde(default = "default_api_key_header")]
    pub header: String,
    pub value: String,
    #[serde(default)]
    pub scheme: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityTaskConfig {
    pub task: String,
    #[serde(default)]
    pub success_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FailureBlockConfig {
    #[serde(default = "default_failure_key")]
    pub key: String,
    #[serde(default = "default_failure_max_failures")]
    pub max_failures: u32,
    #[serde(default = "default_failure_window_secs")]
    pub window_secs: u64,
    #[serde(default = "default_failure_block_secs")]
    pub block_secs: u64,
}

#[derive(Debug, Copy, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Copy, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointAction {
    #[serde(rename = "respond.static")]
    RespondStatic,
    #[serde(rename = "task.run")]
    TaskRun,
    #[serde(rename = "workflow.run")]
    WorkflowRun,
    #[serde(rename = "job.get")]
    JobGet,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskConfig {
    pub name: String,
    #[serde(default)]
    pub mode: TaskMode,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default = "default_inherit_env")]
    pub inherit_env: bool,
    #[serde(default = "default_success_exit_codes")]
    pub success_exit_codes: Vec<i32>,
    #[serde(default = "default_task_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_output_limit_bytes")]
    pub max_stdout_bytes: usize,
    #[serde(default = "default_output_limit_bytes")]
    pub max_stderr_bytes: usize,
    #[serde(default)]
    pub stdin: TaskStdin,
    #[serde(default)]
    pub stdout_format: TaskOutputFormat,
}

#[derive(Debug, Copy, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskMode {
    #[default]
    Sync,
    Async,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskStdin {
    #[default]
    None,
    Body,
    RequestJson,
    Template {
        template: String,
    },
}

#[derive(Debug, Copy, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutputFormat {
    #[default]
    Text,
    Json,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            body_limit_bytes: default_body_limit_bytes(),
        }
    }
}

impl AppConfig {
    pub fn server_config(&self) -> ServerConfig {
        let mut server = ServerConfig::default();
        if let Some(bind) = self.http.bind.clone() {
            server.bind = bind;
        }
        if let Some(port) = self.http.port {
            server.port = port;
        }
        if let Some(limit) = self.http.max_body_bytes.or(self.runtime.max_payload_bytes) {
            server.body_limit_bytes = limit;
        }
        if let Some(bind) = self.artur.server.bind.clone() {
            server.bind = bind;
        }
        if let Some(port) = self.artur.server.port {
            server.port = port;
        }
        if let Some(limit) = self.artur.server.body_limit_bytes {
            server.body_limit_bytes = limit;
        }
        server
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(ArturError::Config(format!(
                "unsupported config version {}; expected version = 1",
                self.version
            )));
        }
        if self.artur.endpoints.is_empty() {
            return Err(ArturError::Config(
                "at least one [[artur.endpoints]] entry is required".to_string(),
            ));
        }

        self.validate_shared_profiles()?;

        let mut endpoint_names = BTreeSet::new();
        let mut endpoint_routes = BTreeSet::new();
        for endpoint in &self.artur.endpoints {
            if endpoint.name.trim().is_empty() {
                return Err(ArturError::Config(
                    "endpoint name cannot be empty".to_string(),
                ));
            }
            if !endpoint_names.insert(endpoint.name.clone()) {
                return Err(ArturError::Config(format!(
                    "duplicate endpoint name {}",
                    endpoint.name
                )));
            }
            if !endpoint.path.starts_with('/') {
                return Err(ArturError::Config(format!(
                    "endpoint {} path must start with /",
                    endpoint.name
                )));
            }
            if !endpoint_routes.insert((
                format!("{:?}", endpoint.method),
                normalize_path_for_validation(&endpoint.path),
            )) {
                return Err(ArturError::Config(format!(
                    "duplicate route {:?} {}",
                    endpoint.method, endpoint.path
                )));
            }
            match endpoint.action {
                EndpointAction::TaskRun => {
                    let task_name = endpoint.task.as_deref().ok_or_else(|| {
                        ArturError::Config(format!(
                            "endpoint {} uses task.run but has no task = ...",
                            endpoint.name
                        ))
                    })?;
                    if self.task_by_name(task_name).is_none() {
                        return Err(ArturError::Config(format!(
                            "endpoint {} references unknown task {}",
                            endpoint.name, task_name
                        )));
                    }
                }
                EndpointAction::WorkflowRun => self.validate_workflow_endpoint(endpoint)?,
                EndpointAction::RespondStatic => {
                    if endpoint.response.is_none() {
                        return Err(ArturError::Config(format!(
                            "endpoint {} uses respond.static but has no [artur.endpoints.response]",
                            endpoint.name
                        )));
                    }
                }
                EndpointAction::JobGet => {
                    if !endpoint.path.contains("{job_id}") && !endpoint.path.contains(":job_id") {
                        return Err(ArturError::Config(format!(
                            "endpoint {} uses job.get but path does not contain {{job_id}}",
                            endpoint.name
                        )));
                    }
                }
            }
            self.validate_security(endpoint)?;
        }

        let mut task_names = BTreeSet::new();
        for task in &self.artur.tasks {
            if task.name.trim().is_empty() {
                return Err(ArturError::Config("task name cannot be empty".to_string()));
            }
            if !task_names.insert(task.name.clone()) {
                return Err(ArturError::Config(format!(
                    "duplicate task name {}",
                    task.name
                )));
            }
            if task.command.trim().is_empty() {
                return Err(ArturError::Config(format!(
                    "task {} command cannot be empty",
                    task.name
                )));
            }
            if task.timeout_ms == 0 {
                return Err(ArturError::Config(format!(
                    "task {} timeout_ms must be greater than 0",
                    task.name
                )));
            }
            if task.success_exit_codes.is_empty() {
                return Err(ArturError::Config(format!(
                    "task {} success_exit_codes cannot be empty",
                    task.name
                )));
            }
            if task.max_stdout_bytes == 0 || task.max_stderr_bytes == 0 {
                return Err(ArturError::Config(format!(
                    "task {} output byte limits must be greater than 0",
                    task.name
                )));
            }
        }
        Ok(())
    }

    pub fn task_by_name(&self, name: &str) -> Option<&TaskConfig> {
        self.artur.tasks.iter().find(|task| task.name == name)
    }

    fn validate_shared_profiles(&self) -> Result<()> {
        for (id, store) in &self.stores {
            if id.trim().is_empty() {
                return Err(ArturError::Config("store id cannot be empty".to_string()));
            }
            if store.url.trim().is_empty() {
                return Err(ArturError::Config(format!(
                    "store {id} url cannot be empty"
                )));
            }
        }
        for (id, profile) in &self.transports.http {
            if id.trim().is_empty() {
                return Err(ArturError::Config(
                    "http transport id cannot be empty".to_string(),
                ));
            }
            if profile.base_url.trim().is_empty() {
                return Err(ArturError::Config(format!(
                    "http transport {id} base_url cannot be empty"
                )));
            }
        }
        Ok(())
    }

    fn validate_security(&self, endpoint: &EndpointConfig) -> Result<()> {
        if let Some(challenge) = &endpoint.security.challenge {
            self.require_task(&challenge.task, endpoint.name.as_str(), "challenge")?;
        }
        if let Some(x402) = &endpoint.security.x402 {
            self.require_task(&x402.task, endpoint.name.as_str(), "x402")?;
        }
        if let Some(api_key) = &endpoint.security.api_key
            && api_key.value.trim().is_empty()
        {
            return Err(ArturError::Config(format!(
                "endpoint {} api_key.value cannot be empty",
                endpoint.name
            )));
        }
        if let Some(block) = &endpoint.security.failure_block
            && (block.max_failures == 0 || block.window_secs == 0 || block.block_secs == 0)
        {
            return Err(ArturError::Config(format!(
                "endpoint {} failure_block limits must be greater than 0",
                endpoint.name
            )));
        }
        Ok(())
    }

    fn validate_workflow_endpoint(&self, endpoint: &EndpointConfig) -> Result<()> {
        if endpoint.steps.is_empty() {
            return Err(ArturError::Config(format!(
                "endpoint {} uses workflow.run but has no [[artur.endpoints.steps]] entries",
                endpoint.name
            )));
        }
        let mut step_ids = BTreeSet::new();
        for step in &endpoint.steps {
            if step.id.trim().is_empty() {
                return Err(ArturError::Config(format!(
                    "endpoint {} has workflow step with empty id",
                    endpoint.name
                )));
            }
            if !step_ids.insert(step.id.clone()) {
                return Err(ArturError::Config(format!(
                    "endpoint {} has duplicate workflow step id {}",
                    endpoint.name, step.id
                )));
            }
        }
        for step in &endpoint.steps {
            for dependency in &step.depends_on {
                if !step_ids.contains(dependency) {
                    return Err(ArturError::Config(format!(
                        "endpoint {} step {} depends on unknown step {}",
                        endpoint.name, step.id, dependency
                    )));
                }
            }
            match step.kind {
                WorkflowStepKind::Task => {
                    let task = step.task.as_deref().ok_or_else(|| {
                        ArturError::Config(format!(
                            "endpoint {} step {} is type=task but has no task",
                            endpoint.name, step.id
                        ))
                    })?;
                    self.require_task(task, endpoint.name.as_str(), &step.id)?;
                }
                WorkflowStepKind::StoreQuery | WorkflowStepKind::StoreExecute => {
                    let store = step.store.as_deref().ok_or_else(|| {
                        ArturError::Config(format!(
                            "endpoint {} step {} is a store operation but has no store",
                            endpoint.name, step.id
                        ))
                    })?;
                    if !self.stores.contains_key(store) {
                        return Err(ArturError::Config(format!(
                            "endpoint {} step {} references unknown store {}",
                            endpoint.name, step.id, store
                        )));
                    }
                    if step.sql.as_deref().unwrap_or_default().trim().is_empty() {
                        return Err(ArturError::Config(format!(
                            "endpoint {} step {} has empty sql",
                            endpoint.name, step.id
                        )));
                    }
                }
                WorkflowStepKind::HttpRequest => {
                    if let Some(transport) = &step.transport
                        && !self.transports.http.contains_key(transport)
                    {
                        return Err(ArturError::Config(format!(
                            "endpoint {} step {} references unknown http transport {}",
                            endpoint.name, step.id, transport
                        )));
                    }
                    if step.transport.is_none()
                        && step.url.as_deref().unwrap_or_default().trim().is_empty()
                    {
                        return Err(ArturError::Config(format!(
                            "endpoint {} step {} is type=http.request but has no transport or url",
                            endpoint.name, step.id
                        )));
                    }
                }
                WorkflowStepKind::Respond => {}
            }
        }
        self.validate_workflow_is_acyclic(endpoint)?;
        Ok(())
    }

    fn validate_workflow_is_acyclic(&self, endpoint: &EndpointConfig) -> Result<()> {
        let mut completed = BTreeSet::new();
        let mut pending = endpoint
            .steps
            .iter()
            .map(|step| step.id.clone())
            .collect::<BTreeSet<_>>();

        while !pending.is_empty() {
            let ready = endpoint
                .steps
                .iter()
                .filter(|step| pending.contains(&step.id))
                .filter(|step| step.depends_on.iter().all(|dep| completed.contains(dep)))
                .map(|step| step.id.clone())
                .collect::<Vec<_>>();

            if ready.is_empty() {
                return Err(ArturError::Config(format!(
                    "endpoint {} workflow has a dependency cycle",
                    endpoint.name
                )));
            }

            for id in ready {
                pending.remove(&id);
                completed.insert(id);
            }
        }
        Ok(())
    }

    fn require_task(&self, task: &str, endpoint: &str, usage: &str) -> Result<()> {
        if self.task_by_name(task).is_none() {
            return Err(ArturError::Config(format!(
                "endpoint {endpoint} {usage} references unknown task {task}"
            )));
        }
        Ok(())
    }
}

pub async fn load_config(location: &str) -> Result<AppConfig> {
    let raw = if location.starts_with("http://") || location.starts_with("https://") {
        reqwest::get(location)
            .await?
            .error_for_status()?
            .text()
            .await?
    } else {
        let path = Path::new(location);
        tokio::fs::read_to_string(path).await?
    };
    let cfg: AppConfig = toml::from_str(&raw)?;
    cfg.validate()?;
    Ok(cfg)
}

fn normalize_path_for_validation(path: &str) -> String {
    let mut out = String::new();
    for segment in path.split('/') {
        if segment.starts_with(':') && segment.len() > 1 {
            out.push('/');
            out.push('{');
            out.push_str(&segment[1..]);
            out.push('}');
        } else if !segment.is_empty() {
            out.push('/');
            out.push_str(segment);
        }
    }
    if out.is_empty() {
        "/".to_string()
    } else if path.ends_with('/') && !out.ends_with('/') {
        out.push('/');
        out
    } else {
        out
    }
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    46796
}

fn default_body_limit_bytes() -> usize {
    1024 * 1024
}

fn default_task_timeout_ms() -> u64 {
    30_000
}

fn default_inherit_env() -> bool {
    true
}

fn default_success_exit_codes() -> Vec<i32> {
    vec![0]
}

fn default_output_limit_bytes() -> usize {
    1024 * 1024
}

fn default_static_status() -> u16 {
    200
}

fn default_static_body() -> Value {
    serde_json::json!({})
}

fn default_workflow_value() -> Value {
    Value::Null
}

fn default_workflow_body() -> Value {
    Value::Null
}

fn default_workflow_include_steps() -> bool {
    true
}

fn default_api_key_header() -> String {
    "authorization".to_string()
}

fn default_failure_key() -> String {
    "{{header.authorization}}".to_string()
}

fn default_failure_max_failures() -> u32 {
    5
}

fn default_failure_window_secs() -> u64 {
    300
}

fn default_failure_block_secs() -> u64 {
    900
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_static_config_in_artur_namespace() {
        let raw = r#"
version = 1

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
        assert_eq!(cfg.server_config().port, 46796);
        assert_eq!(cfg.artur.endpoints.len(), 1);
    }

    #[test]
    fn parses_universal_artur_namespace_and_ignores_other_packages() {
        let raw = r#"
version = 1

[http]
bind = "0.0.0.0"
port = 48080

[stores.artur]
driver = "sqlite"
url = "sqlite://data/artur.db"

[transports.http.ladon]
base_url = "http://ladon:4010/v1"

[ladon]
store = "ladon"

[bria]
ignored = true

[[artur.endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "workflow.run"

[[artur.endpoints.steps]]
id = "reply"
type = "respond"
value = { ok = true }
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.server_config().bind, "0.0.0.0");
        assert_eq!(cfg.server_config().port, 48080);
        assert_eq!(cfg.artur.endpoints[0].steps[0].id, "reply");
        assert!(cfg.stores.contains_key("artur"));
        assert!(cfg.transports.http.contains_key("ladon"));
    }

    #[test]
    fn rejects_missing_artur_endpoints() {
        let raw = r#"
version = 1
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("at least one [[artur.endpoints]]"));
    }

    #[test]
    fn rejects_unknown_task_reference() {
        let raw = r#"
version = 1

[[artur.endpoints]]
name = "run"
method = "POST"
path = "/run"
action = "task.run"
task = "missing"
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_unknown_http_transport_reference() {
        let raw = r#"
version = 1

[[artur.endpoints]]
name = "call"
method = "POST"
path = "/call"
action = "workflow.run"

[[artur.endpoints.steps]]
id = "remote"
type = "http.request"
transport = "missing"
url = "/jobs"
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_unknown_version() {
        let raw = r#"
version = 2

[[artur.endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "respond.static"

[artur.endpoints.response]
body = { ok = true }
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert!(cfg.validate().is_err());
    }
}

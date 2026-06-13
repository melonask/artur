use crate::error::{ArturError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Configuration schema version. Current schema is version = 1.
    pub version: u32,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    #[serde(default)]
    pub processes: Vec<ProcessConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_body_limit_bytes")]
    pub body_limit_bytes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub name: String,
    pub method: HttpMethod,
    pub path: String,
    pub action: EndpointAction,
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub response: Option<StaticResponseConfig>,
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

#[derive(Debug, Copy, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Copy, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointAction {
    #[serde(rename = "respond.static")]
    RespondStatic,
    #[serde(rename = "process.run")]
    ProcessRun,
    #[serde(rename = "job.get")]
    JobGet,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    pub name: String,
    #[serde(default)]
    pub mode: ProcessMode,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default = "default_process_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub stdin: ProcessStdin,
    #[serde(default)]
    pub stdout_format: ProcessOutputFormat,
}

#[derive(Debug, Copy, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessMode {
    #[default]
    Sync,
    Async,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessStdin {
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
pub enum ProcessOutputFormat {
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
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(ArturError::Config(format!(
                "unsupported config version {}; expected version = 1",
                self.version
            )));
        }
        if self.endpoints.is_empty() {
            return Err(ArturError::Config(
                "at least one [[endpoints]] entry is required".to_string(),
            ));
        }

        let mut endpoint_names = BTreeSet::new();
        let mut endpoint_routes = BTreeSet::new();
        for endpoint in &self.endpoints {
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
                EndpointAction::ProcessRun => {
                    let process_name = endpoint.process.as_deref().ok_or_else(|| {
                        ArturError::Config(format!(
                            "endpoint {} uses process.run but has no process = ...",
                            endpoint.name
                        ))
                    })?;
                    if self.process_by_name(process_name).is_none() {
                        return Err(ArturError::Config(format!(
                            "endpoint {} references unknown process {}",
                            endpoint.name, process_name
                        )));
                    }
                }
                EndpointAction::RespondStatic => {
                    if endpoint.response.is_none() {
                        return Err(ArturError::Config(format!(
                            "endpoint {} uses respond.static but has no [endpoints.response]",
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
        }

        let mut process_names = BTreeSet::new();
        for process in &self.processes {
            if process.name.trim().is_empty() {
                return Err(ArturError::Config(
                    "process name cannot be empty".to_string(),
                ));
            }
            if !process_names.insert(process.name.clone()) {
                return Err(ArturError::Config(format!(
                    "duplicate process name {}",
                    process.name
                )));
            }
            if process.command.trim().is_empty() {
                return Err(ArturError::Config(format!(
                    "process {} command cannot be empty",
                    process.name
                )));
            }
            if process.timeout_ms == 0 {
                return Err(ArturError::Config(format!(
                    "process {} timeout_ms must be greater than 0",
                    process.name
                )));
            }
        }
        Ok(())
    }

    pub fn process_by_name(&self, name: &str) -> Option<&ProcessConfig> {
        self.processes.iter().find(|process| process.name == name)
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

fn default_process_timeout_ms() -> u64 {
    30_000
}

fn default_static_status() -> u16 {
    200
}

fn default_static_body() -> Value {
    serde_json::json!({})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_static_config() {
        let raw = r#"
version = 1

[[endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "respond.static"

[endpoints.response]
body = { ok = true }
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.server.port, 46796);
        assert_eq!(cfg.endpoints.len(), 1);
    }

    #[test]
    fn rejects_unknown_process_reference() {
        let raw = r#"
version = 1

[[endpoints]]
name = "run"
method = "POST"
path = "/run"
action = "process.run"
process = "missing"
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_unknown_version() {
        let raw = r#"
version = 2

[[endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "respond.static"

[endpoints.response]
body = { ok = true }
"#;
        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert!(cfg.validate().is_err());
    }
}

use crate::{
    AppConfig,
    config::{
        EndpointConfig, HttpMethod, HttpTransportConfig, WorkflowStepConfig, WorkflowStepKind,
    },
    error::{ArturError, Result},
    process::{RequestContext, render_json_value, render_template, run_task},
    store::run_store_step,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::task::JoinSet;

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowOutput {
    pub ok: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub steps: BTreeMap<String, Value>,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpStepOutput {
    pub ok: bool,
    pub status: u16,
    pub url: String,
    pub body: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_parse_error: Option<String>,
}

pub async fn run_workflow(
    config: Arc<AppConfig>,
    endpoint: EndpointConfig,
    request: RequestContext,
) -> Result<WorkflowOutput> {
    let mut pending = endpoint
        .steps
        .iter()
        .cloned()
        .map(|step| (step.id.clone(), step))
        .collect::<BTreeMap<_, _>>();
    let mut completed = BTreeMap::new();

    while !pending.is_empty() {
        let ready_ids = pending
            .iter()
            .filter(|(_, step)| {
                step.depends_on
                    .iter()
                    .all(|dependency| completed.contains_key(dependency))
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();

        if ready_ids.is_empty() {
            return Err(ArturError::Config(format!(
                "endpoint {} workflow has a dependency cycle or unsatisfied dependency",
                endpoint.name
            )));
        }

        let snapshot = completed.clone();
        let mut tasks = JoinSet::new();
        for id in ready_ids {
            let Some(step) = pending.remove(&id) else {
                continue;
            };
            let step_id = step.id.clone();
            let continue_on_error = step.continue_on_error;
            let request_for_step = request.with_steps(snapshot.clone());
            let config_for_step = config.clone();
            tasks.spawn(async move {
                let result = run_step(config_for_step, step, request_for_step).await;
                (step_id, continue_on_error, result)
            });
        }

        while let Some(joined) = tasks.join_next().await {
            let (step_id, continue_on_error, result) = joined
                .map_err(|err| ArturError::Process(format!("workflow step join error: {err}")))?;
            match result {
                Ok(value) => {
                    completed.insert(step_id, value);
                }
                Err(err) if continue_on_error => {
                    completed.insert(
                        step_id,
                        serde_json::json!({
                            "ok": false,
                            "error": err.code(),
                            "message": err.to_string(),
                        }),
                    );
                }
                Err(err) => return Err(err),
            }
        }
    }

    let request_with_steps = request.with_steps(completed.clone());
    let result = if endpoint.result.body.is_null() {
        last_respond_value(&endpoint.steps, &completed).unwrap_or_else(|| Value::Object(Map::new()))
    } else {
        render_json_value(&endpoint.result.body, &request_with_steps)?
    };

    Ok(WorkflowOutput {
        ok: true,
        steps: if endpoint.result.include_steps {
            completed
        } else {
            BTreeMap::new()
        },
        result,
    })
}

async fn run_step(
    config: Arc<AppConfig>,
    step: WorkflowStepConfig,
    request: RequestContext,
) -> Result<Value> {
    match step.kind {
        WorkflowStepKind::Task => run_task_step(config, step, request).await,
        WorkflowStepKind::StoreQuery | WorkflowStepKind::StoreExecute => {
            run_database_step(config, step, request).await
        }
        WorkflowStepKind::HttpRequest => run_http_request_step(config, step, request).await,
        WorkflowStepKind::Respond => Ok(serde_json::json!({
            "ok": true,
            "value": render_json_value(&step.value, &request)?,
        })),
    }
}

async fn run_task_step(
    config: Arc<AppConfig>,
    step: WorkflowStepConfig,
    request: RequestContext,
) -> Result<Value> {
    let task_name = step
        .task
        .as_deref()
        .ok_or_else(|| ArturError::Config(format!("workflow step {} has no task", step.id)))?;
    let task = config
        .task_by_name(task_name)
        .cloned()
        .ok_or_else(|| ArturError::Config(format!("unknown task {task_name}")))?;
    let output = run_task(&task, &request).await?;
    if !output.ok {
        return Err(ArturError::Process(format!(
            "workflow step {} task {} failed: {}",
            step.id, task_name, output.stderr
        )));
    }
    serde_json::to_value(output).map_err(ArturError::from)
}

async fn run_database_step(
    config: Arc<AppConfig>,
    step: WorkflowStepConfig,
    request: RequestContext,
) -> Result<Value> {
    let store_name = step
        .store
        .as_deref()
        .ok_or_else(|| ArturError::Config(format!("workflow step {} has no store", step.id)))?;
    let store = config
        .stores
        .get(store_name)
        .ok_or_else(|| ArturError::Config(format!("unknown store {store_name}")))?;
    let output = run_store_step(store_name, store, &step, &request).await?;
    serde_json::to_value(output).map_err(ArturError::from)
}

async fn run_http_request_step(
    config: Arc<AppConfig>,
    step: WorkflowStepConfig,
    request: RequestContext,
) -> Result<Value> {
    let transport = step
        .transport
        .as_ref()
        .and_then(|id| config.transports.http.get(id));
    let url = render_http_url(&step, transport, &request)?;
    let method = step.method.unwrap_or(HttpMethod::Get);
    let timeout_ms = step
        .timeout_ms
        .or_else(|| transport.and_then(|transport| transport.timeout_ms))
        .unwrap_or(30_000);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()?;
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|err| ArturError::Config(format!("invalid HTTP method: {err}")))?;
    let mut builder = client.request(reqwest_method, &url);

    if let Some(transport) = transport {
        for (name, value) in &transport.headers {
            builder = builder.header(name.as_str(), render_template(value, &request)?);
        }
    }
    for (name, value) in &step.headers {
        builder = builder.header(name.as_str(), render_template(value, &request)?);
    }

    if !step.body.is_null() {
        let rendered_body = render_json_value(&step.body, &request)?;
        if !has_content_type(&step, transport) {
            builder = builder.header("content-type", "application/json");
        }
        let payload = match rendered_body {
            Value::String(value) => value,
            value => serde_json::to_string(&value)?,
        };
        builder = builder.body(payload);
    }

    let response = builder.send().await?;
    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.to_string(), value.to_string()))
        })
        .collect::<BTreeMap<_, _>>();
    let body = response.text().await?;
    let (json, json_parse_error) = if body.trim().is_empty() {
        (None, None)
    } else {
        match serde_json::from_str(&body) {
            Ok(value) => (Some(value), None),
            Err(err) => (None, Some(err.to_string())),
        }
    };
    let output = HttpStepOutput {
        ok: status.is_success(),
        status: status.as_u16(),
        url,
        body,
        headers,
        json,
        json_parse_error,
    };
    if !output.ok {
        return Err(ArturError::Process(format!(
            "workflow step {} http.request failed with status {}",
            step.id, output.status
        )));
    }
    serde_json::to_value(output).map_err(ArturError::from)
}

fn render_http_url(
    step: &WorkflowStepConfig,
    transport: Option<&HttpTransportConfig>,
    request: &RequestContext,
) -> Result<String> {
    let suffix = step.url.as_deref().unwrap_or_default();
    let rendered_suffix = render_template(suffix, request)?;
    if rendered_suffix.starts_with("http://") || rendered_suffix.starts_with("https://") {
        return Ok(rendered_suffix);
    }
    let Some(transport) = transport else {
        if rendered_suffix.trim().is_empty() {
            return Err(ArturError::Config(format!(
                "workflow step {} has no http url",
                step.id
            )));
        }
        return Ok(rendered_suffix);
    };
    let base = transport.base_url.trim_end_matches('/');
    let suffix = rendered_suffix.trim_start_matches('/');
    if suffix.is_empty() {
        Ok(base.to_string())
    } else {
        Ok(format!("{base}/{suffix}"))
    }
}

fn has_content_type(step: &WorkflowStepConfig, transport: Option<&HttpTransportConfig>) -> bool {
    step.headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("content-type"))
        || transport
            .map(|transport| {
                transport
                    .headers
                    .keys()
                    .any(|key| key.eq_ignore_ascii_case("content-type"))
            })
            .unwrap_or(false)
}

fn last_respond_value(
    configured_steps: &[WorkflowStepConfig],
    step_outputs: &BTreeMap<String, Value>,
) -> Option<Value> {
    configured_steps.iter().rev().find_map(|step| {
        if step.kind == WorkflowStepKind::Respond {
            step_outputs.get(&step.id)?.get("value").cloned()
        } else {
            None
        }
    })
}

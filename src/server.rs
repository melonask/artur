use crate::{
    AppConfig,
    api::HealthResponse,
    config::{EndpointAction, EndpointConfig, HttpMethod},
    error::{ArturError, Result},
    process::{
        JobRecord, JobStore, RequestContext, TaskRunResponse, hashmap_to_btree,
        header_map_to_btree, run_or_enqueue,
    },
    security::{SecurityState, authorize_endpoint},
    workflow::run_workflow,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, OriginalUri, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header::HeaderName},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub jobs: JobStore,
    pub security: SecurityState,
}

struct EndpointRequest {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    query: HashMap<String, String>,
    params: Option<Path<HashMap<String, String>>>,
    body: Bytes,
}

pub async fn build_router(config: AppConfig) -> Result<Router> {
    config.validate()?;
    let state = AppState {
        config: Arc::new(config.clone()),
        jobs: JobStore::default(),
        security: SecurityState::default(),
    };

    let mut router = Router::new().route("/healthz", get(health));
    for endpoint in config.artur.endpoints.clone() {
        router = register_endpoint(router, endpoint)?;
    }

    let router = router.layer(DefaultBodyLimit::max(
        config.server_config().body_limit_bytes,
    ));
    Ok(router.with_state(state))
}

fn register_endpoint(
    router: Router<AppState>,
    endpoint: EndpointConfig,
) -> Result<Router<AppState>> {
    let path = normalize_axum_path(&endpoint.path);
    let router = match endpoint.method {
        HttpMethod::Get => {
            let endpoint_for_handler = endpoint.clone();
            router.route(
                &path,
                get(
                    move |state: State<AppState>,
                          method: Method,
                          uri: OriginalUri,
                          headers: HeaderMap,
                          query: Query<HashMap<String, String>>,
                          params: Option<Path<HashMap<String, String>>>,
                          body: Bytes| {
                        handle_configured_endpoint(
                            state,
                            endpoint_for_handler.clone(),
                            EndpointRequest {
                                method,
                                uri: uri.0,
                                headers,
                                query: query.0,
                                params,
                                body,
                            },
                        )
                    },
                ),
            )
        }
        HttpMethod::Post => {
            let endpoint_for_handler = endpoint.clone();
            router.route(
                &path,
                post(
                    move |state: State<AppState>,
                          method: Method,
                          uri: OriginalUri,
                          headers: HeaderMap,
                          query: Query<HashMap<String, String>>,
                          params: Option<Path<HashMap<String, String>>>,
                          body: Bytes| {
                        handle_configured_endpoint(
                            state,
                            endpoint_for_handler.clone(),
                            EndpointRequest {
                                method,
                                uri: uri.0,
                                headers,
                                query: query.0,
                                params,
                                body,
                            },
                        )
                    },
                ),
            )
        }
        HttpMethod::Put => {
            let endpoint_for_handler = endpoint.clone();
            router.route(
                &path,
                put(
                    move |state: State<AppState>,
                          method: Method,
                          uri: OriginalUri,
                          headers: HeaderMap,
                          query: Query<HashMap<String, String>>,
                          params: Option<Path<HashMap<String, String>>>,
                          body: Bytes| {
                        handle_configured_endpoint(
                            state,
                            endpoint_for_handler.clone(),
                            EndpointRequest {
                                method,
                                uri: uri.0,
                                headers,
                                query: query.0,
                                params,
                                body,
                            },
                        )
                    },
                ),
            )
        }
        HttpMethod::Patch => {
            let endpoint_for_handler = endpoint.clone();
            router.route(
                &path,
                patch(
                    move |state: State<AppState>,
                          method: Method,
                          uri: OriginalUri,
                          headers: HeaderMap,
                          query: Query<HashMap<String, String>>,
                          params: Option<Path<HashMap<String, String>>>,
                          body: Bytes| {
                        handle_configured_endpoint(
                            state,
                            endpoint_for_handler.clone(),
                            EndpointRequest {
                                method,
                                uri: uri.0,
                                headers,
                                query: query.0,
                                params,
                                body,
                            },
                        )
                    },
                ),
            )
        }
        HttpMethod::Delete => {
            let endpoint_for_handler = endpoint.clone();
            router.route(
                &path,
                delete(
                    move |state: State<AppState>,
                          method: Method,
                          uri: OriginalUri,
                          headers: HeaderMap,
                          query: Query<HashMap<String, String>>,
                          params: Option<Path<HashMap<String, String>>>,
                          body: Bytes| {
                        handle_configured_endpoint(
                            state,
                            endpoint_for_handler.clone(),
                            EndpointRequest {
                                method,
                                uri: uri.0,
                                headers,
                                query: query.0,
                                params,
                                body,
                            },
                        )
                    },
                ),
            )
        }
    };
    Ok(router)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        version: state.config.version,
    })
}

async fn handle_configured_endpoint(
    State(state): State<AppState>,
    endpoint: EndpointConfig,
    request_parts: EndpointRequest,
) -> Result<Response> {
    if let Some(limit) = endpoint.body_limit_bytes
        && request_parts.body.len() > limit
    {
        return Err(ArturError::PayloadTooLarge(format!(
            "endpoint {} body exceeded {} bytes",
            endpoint.name, limit
        )));
    }

    let params = request_parts
        .params
        .map(|Path(params)| params)
        .unwrap_or_default();
    let request = RequestContext::from_parts(
        request_parts.method.to_string(),
        request_parts.uri.to_string(),
        request_parts.uri.path().to_string(),
        hashmap_to_btree(params.clone()),
        hashmap_to_btree(request_parts.query),
        header_map_to_btree(&request_parts.headers),
        request_parts.body,
    );

    authorize_endpoint(
        state.config.clone(),
        state.security.clone(),
        &endpoint,
        &request,
    )
    .await?;

    match endpoint.action {
        EndpointAction::RespondStatic => respond_static(endpoint),
        EndpointAction::TaskRun => run_task_endpoint(state, endpoint, request).await,
        EndpointAction::WorkflowRun => run_workflow_endpoint(state, endpoint, request).await,
        EndpointAction::JobGet => get_job_by_path(state, params).await,
    }
}

fn respond_static(endpoint: EndpointConfig) -> Result<Response> {
    let response_cfg = endpoint.response.ok_or_else(|| {
        ArturError::Config(format!(
            "endpoint {} uses respond.static but has no response config",
            endpoint.name
        ))
    })?;
    respond_json(
        response_cfg.status,
        response_cfg.body,
        response_cfg.headers,
        &endpoint.name,
    )
}

async fn run_task_endpoint(
    state: AppState,
    endpoint: EndpointConfig,
    request: RequestContext,
) -> Result<Response> {
    let task_name = endpoint
        .task
        .ok_or_else(|| ArturError::Config("task.run endpoint is missing task".to_string()))?;
    let task = state
        .config
        .task_by_name(&task_name)
        .cloned()
        .ok_or_else(|| ArturError::Config(format!("task {task_name} is not configured")))?;

    let output: TaskRunResponse = run_or_enqueue(task, request, state.jobs.clone()).await?;
    Ok(Json(output).into_response())
}

async fn run_workflow_endpoint(
    state: AppState,
    endpoint: EndpointConfig,
    request: RequestContext,
) -> Result<Response> {
    let output = run_workflow(state.config.clone(), endpoint.clone(), request).await?;
    let body = if endpoint.result.body.is_null() || endpoint.result.include_steps {
        serde_json::to_value(output)?
    } else {
        output.result
    };
    respond_json(
        endpoint.result.status,
        body,
        endpoint.result.headers,
        &endpoint.name,
    )
}

async fn get_job_by_path(state: AppState, params: HashMap<String, String>) -> Result<Response> {
    let job_id = params
        .get("job_id")
        .ok_or_else(|| ArturError::Request("missing job_id path parameter".to_string()))?;
    let job: JobRecord = state
        .jobs
        .get(job_id)
        .await
        .ok_or_else(|| ArturError::NotFound(format!("job {job_id} not found")))?;
    Ok(Json(job).into_response())
}

fn respond_json(
    status: u16,
    body: Value,
    headers: HashMapLike,
    endpoint_name: &str,
) -> Result<Response> {
    let status = StatusCode::from_u16(status).map_err(|err| {
        ArturError::Config(format!(
            "endpoint {endpoint_name} has invalid response status: {err}"
        ))
    })?;
    let mut response = (status, Json(body)).into_response();
    for (name, value) in headers {
        let name_for_error = name.clone();
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            ArturError::Config(format!(
                "endpoint {endpoint_name} has invalid response header name {name_for_error}: {err}"
            ))
        })?;
        let value = HeaderValue::from_str(&value).map_err(|err| {
            ArturError::Config(format!(
                "endpoint {endpoint_name} has invalid response header value for {name}: {err}"
            ))
        })?;
        response.headers_mut().insert(name, value);
    }
    Ok(response)
}

type HashMapLike = std::collections::BTreeMap<String, String>;

fn normalize_axum_path(path: &str) -> String {
    // `:name` was common in older routers. Axum 0.8 uses `{name}`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_colon_params() {
        assert_eq!(normalize_axum_path("/jobs/:job_id"), "/jobs/{job_id}");
        assert_eq!(normalize_axum_path("/space/"), "/space/");
    }
}

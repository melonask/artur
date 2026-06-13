use crate::{
    AppConfig,
    api::HealthResponse,
    config::{EndpointAction, EndpointConfig, HttpMethod},
    error::{ArturError, Result},
    process::{
        JobRecord, JobStore, ProcessRunResponse, RequestContext, hashmap_to_btree,
        header_map_to_btree, run_or_enqueue,
    },
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, OriginalUri, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header::HeaderName},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub jobs: JobStore,
}

struct ProcessEndpointRequest {
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
    };

    let mut router = Router::new().route("/healthz", get(health));
    for endpoint in config.endpoints.clone() {
        router = register_endpoint(router, endpoint)?;
    }

    let router = router.layer(DefaultBodyLimit::max(config.server.body_limit_bytes));
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
                            method,
                            uri,
                            headers,
                            query,
                            params,
                            body,
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
                            method,
                            uri,
                            headers,
                            query,
                            params,
                            body,
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
                            method,
                            uri,
                            headers,
                            query,
                            params,
                            body,
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
                            method,
                            uri,
                            headers,
                            query,
                            params,
                            body,
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
                            method,
                            uri,
                            headers,
                            query,
                            params,
                            body,
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

#[allow(clippy::too_many_arguments)]
async fn handle_configured_endpoint(
    State(state): State<AppState>,
    endpoint: EndpointConfig,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    params: Option<Path<HashMap<String, String>>>,
    body: Bytes,
) -> Result<Response> {
    match endpoint.action {
        EndpointAction::RespondStatic => respond_static(endpoint),
        EndpointAction::ProcessRun => {
            run_process_endpoint(
                state,
                endpoint,
                ProcessEndpointRequest {
                    method,
                    uri,
                    headers,
                    query,
                    params,
                    body,
                },
            )
            .await
        }
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
    let status = StatusCode::from_u16(response_cfg.status).map_err(|err| {
        ArturError::Config(format!(
            "endpoint {} has invalid response status {}: {err}",
            endpoint.name, response_cfg.status
        ))
    })?;
    let mut response = (status, Json(response_cfg.body)).into_response();
    for (name, value) in response_cfg.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            ArturError::Config(format!(
                "endpoint {} has invalid response header name {name}: {err}",
                endpoint.name
            ))
        })?;
        let value = HeaderValue::from_str(&value).map_err(|err| {
            ArturError::Config(format!(
                "endpoint {} has invalid response header value for {name}: {err}",
                endpoint.name
            ))
        })?;
        response.headers_mut().insert(name, value);
    }
    Ok(response)
}

async fn run_process_endpoint(
    state: AppState,
    endpoint: EndpointConfig,
    request_parts: ProcessEndpointRequest,
) -> Result<Response> {
    let process_name = endpoint
        .process
        .ok_or_else(|| ArturError::Config("process.run endpoint is missing process".to_string()))?;
    let process = state
        .config
        .process_by_name(&process_name)
        .cloned()
        .ok_or_else(|| ArturError::Config(format!("process {process_name} is not configured")))?;

    let params = request_parts
        .params
        .map(|Path(params)| params)
        .unwrap_or_default();
    let request = RequestContext::from_parts(
        request_parts.method.to_string(),
        request_parts.uri.to_string(),
        request_parts.uri.path().to_string(),
        hashmap_to_btree(params),
        hashmap_to_btree(request_parts.query),
        header_map_to_btree(&request_parts.headers),
        request_parts.body,
    );
    let output: ProcessRunResponse = run_or_enqueue(process, request, state.jobs.clone()).await?;
    Ok(Json(output).into_response())
}

async fn get_job_by_path(
    state: AppState,
    params: Option<Path<HashMap<String, String>>>,
) -> Result<Response> {
    let params = params.map(|Path(params)| params).unwrap_or_default();
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

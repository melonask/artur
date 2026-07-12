use crate::{
    AppConfig,
    api::HealthResponse,
    config::{ClientIpHeader, EndpointAction, EndpointConfig, HttpMethod},
    error::{ArturError, Result},
    idempotency::{Claim, capture, claim, complete, key as idempotency_key, release, replay},
    process::{
        JobRecord, JobStore, RequestContext, TaskRunResponse, hashmap_to_btree,
        header_map_to_btree, run_or_enqueue,
    },
    rate_limit,
    security::{SecurityState, authorize_endpoint},
    workflow::run_workflow,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, OriginalUri, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header::HeaderName},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
use ipnet::IpNet;
use serde_json::Value;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub jobs: JobStore,
    pub security: SecurityState,
    pub concurrency: Arc<HashMap<String, Arc<Semaphore>>>,
}

struct EndpointRequest {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    query: HashMap<String, String>,
    params: Option<Path<HashMap<String, String>>>,
    body: Bytes,
    peer: SocketAddr,
}

pub async fn build_router(config: AppConfig) -> Result<Router> {
    config.validate()?;
    let state = AppState {
        config: Arc::new(config.clone()),
        jobs: JobStore::default(),
        security: SecurityState::default(),
        concurrency: Arc::new(
            config
                .artur
                .endpoints
                .iter()
                .filter_map(|e| {
                    e.restrictions
                        .max_concurrency
                        .map(|limit| (e.name.clone(), Arc::new(Semaphore::new(limit))))
                })
                .collect(),
        ),
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
                          ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
                                peer,
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
                          ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
                                peer,
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
                          ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
                                peer,
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
                          ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
                                peer,
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
                          ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
                                peer,
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
    validate_metadata(&endpoint, &request_parts)?;
    let client_ip = resolve_client_ip(&state.config, request_parts.peer, &request_parts.headers)?;
    let deadline = endpoint.restrictions.timeout_ms.map(Duration::from_millis);
    let operation = async {
        let _permit = acquire_concurrency(&state, &endpoint)?;
        let rate_config = endpoint.security.rate_limit.clone();
        let rate = apply_rate_limit(&state, &endpoint, &request_parts, &client_ip).await?;
        let mut response = handle_after_guards(state, endpoint, request_parts, client_ip).await?;
        if let (Some(rate), Some(rate_config)) = (rate, rate_config.as_ref()) {
            add_rate_limit_headers(&mut response, &rate, rate_config);
        }
        Ok(response)
    };
    if let Some(deadline) = deadline {
        tokio::time::timeout(deadline, operation)
            .await
            .map_err(|_| ArturError::Timeout("endpoint deadline exceeded".to_string()))?
    } else {
        operation.await
    }
}

async fn handle_after_guards(
    state: AppState,
    endpoint: EndpointConfig,
    request_parts: EndpointRequest,
    client_ip: String,
) -> Result<Response> {
    let request_idempotency_key = endpoint
        .idempotency
        .as_ref()
        .map(|config| idempotency_key(&request_parts.headers, config))
        .transpose()?
        .flatten();

    let params = request_parts
        .params
        .map(|Path(params)| params)
        .unwrap_or_default();
    let request = RequestContext::from_parts(
        // Client identity is only populated after trusted-proxy resolution.
        request_parts.method.to_string(),
        request_parts.uri.to_string(),
        request_parts.uri.path().to_string(),
        hashmap_to_btree(params.clone()),
        hashmap_to_btree(request_parts.query),
        header_map_to_btree(&request_parts.headers),
        request_parts.body,
    );
    let request = RequestContext {
        client: crate::process::ClientContext { ip: client_ip },
        ..request
    };

    authorize_endpoint(
        state.config.clone(),
        state.security.clone(),
        &endpoint,
        &request,
    )
    .await?;

    let idempotency = endpoint.idempotency.clone();
    if let Some(idempotency) = idempotency {
        let Some(key) = request_idempotency_key else {
            return run_endpoint_action(state, endpoint, request, params).await;
        };
        let store = state
            .config
            .stores
            .get(&idempotency.store)
            .cloned()
            .ok_or_else(|| {
                ArturError::Config(format!(
                    "idempotency store {} is not configured",
                    idempotency.store
                ))
            })?;
        match claim(&endpoint.name, &key, &request, &idempotency, &store).await? {
            Claim::Replay(response) => return replay(response),
            Claim::Claimed => {}
        }
        let response = run_endpoint_action(state, endpoint.clone(), request, params).await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                release(&endpoint.name, &key, &store).await?;
                return Err(error);
            }
        };
        let (response, stored) = match capture(response, idempotency.max_response_bytes).await {
            Ok(captured) => captured,
            Err(error) => {
                release(&endpoint.name, &key, &store).await?;
                return Err(error);
            }
        };
        complete(&endpoint.name, &key, &store, stored).await?;
        return Ok(response);
    }

    run_endpoint_action(state, endpoint, request, params).await
}

fn acquire_concurrency(
    state: &AppState,
    endpoint: &EndpointConfig,
) -> Result<Option<OwnedSemaphorePermit>> {
    state
        .concurrency
        .get(&endpoint.name)
        .map(|semaphore| {
            semaphore
                .clone()
                .try_acquire_owned()
                .map(Some)
                .map_err(|_| {
                    ArturError::TooManyRequests("endpoint concurrency limit reached".to_string())
                })
        })
        .transpose()
        .map(|permit| permit.flatten())
}

async fn apply_rate_limit(
    state: &AppState,
    endpoint: &EndpointConfig,
    parts: &EndpointRequest,
    client_ip: &str,
) -> Result<Option<rate_limit::RateLimitResult>> {
    let Some(rate) = &endpoint.security.rate_limit else {
        return Ok(None);
    };
    let request = RequestContext::from_parts(
        parts.method.to_string(),
        parts.uri.to_string(),
        parts.uri.path().to_string(),
        Default::default(),
        Default::default(),
        header_map_to_btree(&parts.headers),
        Bytes::new(),
    )
    .with_client_ip(client_ip.to_string());
    let key = crate::process::render_template(&rate.key, &request)?;
    let store = state
        .config
        .stores
        .get(&rate.store)
        .ok_or_else(|| ArturError::Config("rate limit store is not configured".to_string()))?;
    let result = rate_limit::check(&endpoint.name, &key, rate, store).await?;
    if result.allowed {
        Ok(Some(result))
    } else {
        Err(ArturError::RateLimited {
            retry_after: result.retry_after,
            limit: rate.requests,
            window_secs: rate.window_secs,
        })
    }
}

fn add_rate_limit_headers(
    response: &mut Response,
    rate: &rate_limit::RateLimitResult,
    config: &crate::config::RateLimitConfig,
) {
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&format!(
        "\"{}\";r={};t={}",
        config.requests, rate.remaining, rate.retry_after
    )) {
        headers.insert("ratelimit", value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!(
        "\"{}\";q={};w={}",
        config.requests, config.requests, config.window_secs
    )) {
        headers.insert("ratelimit-policy", value);
    }
}

fn validate_metadata(endpoint: &EndpointConfig, parts: &EndpointRequest) -> Result<()> {
    for header in &endpoint.restrictions.required_headers {
        if !parts.headers.contains_key(header) {
            return Err(ArturError::Request(format!(
                "missing required header {header}"
            )));
        }
    }
    if !parts.body.is_empty() && !endpoint.restrictions.allowed_content_types.is_empty() {
        let content_type = parts
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::trim);
        if !content_type.is_some_and(|v| {
            endpoint
                .restrictions
                .allowed_content_types
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(v))
        }) {
            return Err(ArturError::UnsupportedMediaType(
                "request content type is not allowed".to_string(),
            ));
        }
    }
    Ok(())
}

fn resolve_client_ip(config: &AppConfig, peer: SocketAddr, headers: &HeaderMap) -> Result<String> {
    let peer = peer.ip();
    let cfg = &config.artur.server.client_ip;
    let trusted = cfg
        .trusted_proxy_cidrs
        .iter()
        .map(|cidr| {
            IpNet::from_str(cidr)
                .map_err(|_| ArturError::Config("invalid trusted proxy CIDR".to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    if cfg.header.is_none() || !trusted.iter().any(|net| net.contains(&peer)) {
        return Ok(peer.to_string());
    }
    let raw = match cfg.header {
        Some(ClientIpHeader::XForwardedFor) => headers.get("x-forwarded-for"),
        Some(ClientIpHeader::Forwarded) => headers.get("forwarded"),
        None => None,
    }
    .ok_or_else(|| ArturError::Request("configured forwarding header is missing".to_string()))?
    .to_str()
    .map_err(|_| ArturError::Request("configured forwarding header is malformed".to_string()))?;
    let header = cfg.header.ok_or_else(|| {
        ArturError::Config("trusted proxy CIDRs require a forwarding header".to_string())
    })?;
    let chain = match header {
        ClientIpHeader::XForwardedFor => raw
            .split(',')
            .map(|v| {
                v.trim().parse::<IpAddr>().map_err(|_| {
                    ArturError::Request("configured forwarding header is malformed".to_string())
                })
            })
            .collect::<Result<Vec<_>>>()?,
        ClientIpHeader::Forwarded => raw
            .split(',')
            .map(|v| {
                v.split(';')
                    .find_map(|p| p.trim().strip_prefix("for="))
                    .map(|v| {
                        v.trim_matches('"')
                            .trim_matches(|c| c == '[' || c == ']')
                            .parse::<IpAddr>()
                            .map_err(|_| {
                                ArturError::Request(
                                    "configured forwarding header is malformed".to_string(),
                                )
                            })
                    })
                    .ok_or_else(|| {
                        ArturError::Request("configured forwarding header is malformed".to_string())
                    })?
            })
            .collect::<Result<Vec<_>>>()?,
    };
    Ok(chain
        .into_iter()
        .rev()
        .find(|ip| !trusted.iter().any(|net| net.contains(ip)))
        .unwrap_or(peer)
        .to_string())
}

async fn run_endpoint_action(
    state: AppState,
    endpoint: EndpointConfig,
    request: RequestContext,
    params: HashMap<String, String>,
) -> Result<Response> {
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
    use crate::config::AppConfig;
    use axum::http::HeaderValue;
    use axum::response::IntoResponse;

    fn client_config() -> AppConfig {
        toml::from_str(
            r#"version=1
[artur.server.client_ip]
trusted_proxy_cidrs=["10.0.0.0/8"]
header="x-forwarded-for"
[[artur.endpoints]]
name="x"
method="GET"
path="/x"
action="respond.static"
[artur.endpoints.response]
body={}"#,
        )
        .unwrap()
    }

    #[test]
    fn direct_peer_is_client_ip() {
        let cfg = client_config();
        assert_eq!(
            resolve_client_ip(&cfg, "192.0.2.8:99".parse().unwrap(), &HeaderMap::new()).unwrap(),
            "192.0.2.8"
        );
    }

    #[test]
    fn spoofed_xff_from_untrusted_peer_is_ignored() {
        let cfg = client_config();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.4"));
        assert_eq!(
            resolve_client_ip(&cfg, "192.0.2.8:99".parse().unwrap(), &headers).unwrap(),
            "192.0.2.8"
        );
    }

    #[test]
    fn trusted_xff_walks_right_to_left() {
        let cfg = client_config();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.2, 10.1.1.2, 10.2.2.2"),
        );
        assert_eq!(
            resolve_client_ip(&cfg, "10.3.3.3:99".parse().unwrap(), &headers).unwrap(),
            "198.51.100.2"
        );
    }

    #[test]
    fn malformed_trusted_xff_is_rejected() {
        let cfg = client_config();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        assert!(resolve_client_ip(&cfg, "10.3.3.3:99".parse().unwrap(), &headers).is_err());
    }

    #[test]
    fn validates_content_type_required_headers_and_bodyless_requests() {
        let endpoint = EndpointConfig {
            name: "x".into(),
            method: HttpMethod::Post,
            path: "/x".into(),
            action: EndpointAction::RespondStatic,
            task: None,
            response: None,
            security: Default::default(),
            body_limit_bytes: None,
            restrictions: crate::config::EndpointRestrictions {
                allowed_content_types: vec!["application/json".into()],
                required_headers: vec!["x-id".into()],
                timeout_ms: None,
                max_concurrency: None,
            },
            idempotency: None,
            steps: vec![],
            result: Default::default(),
        };
        let base = |body: Bytes, headers: HeaderMap| EndpointRequest {
            method: Method::POST,
            uri: "/x".parse().unwrap(),
            headers,
            query: Default::default(),
            params: None,
            body,
            peer: "127.0.0.1:1".parse().unwrap(),
        };
        let headers = |content_type| {
            let mut headers = HeaderMap::new();
            headers.insert("x-id", HeaderValue::from_static("x"));
            if content_type {
                headers.insert("content-type", HeaderValue::from_static("application/json"));
            }
            headers
        };
        assert!(validate_metadata(&endpoint, &base(Bytes::new(), headers(false))).is_ok());
        assert!(matches!(
            validate_metadata(&endpoint, &base(Bytes::from_static(b"x"), headers(false))),
            Err(ArturError::UnsupportedMediaType(_))
        ));
        assert!(
            validate_metadata(&endpoint, &base(Bytes::from_static(b"x"), headers(true))).is_ok()
        );
        assert!(validate_metadata(&endpoint, &base(Bytes::new(), HeaderMap::new())).is_err());
    }

    #[test]
    fn problem_errors_have_required_status_and_content_type() {
        for (error, status) in [
            (
                ArturError::UnsupportedMediaType("bad".into()),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
            (
                ArturError::Timeout("late".into()),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            (
                ArturError::RateLimited {
                    retry_after: 10,
                    limit: 2,
                    window_secs: 60,
                },
                StatusCode::TOO_MANY_REQUESTS,
            ),
        ] {
            let response = error.into_response();
            assert_eq!(response.status(), status);
            assert_eq!(
                response.headers()["content-type"],
                "application/problem+json"
            );
        }
    }

    #[tokio::test]
    async fn endpoint_timeout_maps_to_504() {
        let result = tokio::time::timeout(
            Duration::from_millis(1),
            tokio::time::sleep(Duration::from_millis(20)),
        )
        .await;
        let response = result
            .map_err(|_| ArturError::Timeout("endpoint deadline exceeded".into()))
            .unwrap_err()
            .into_response();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
    }

    #[test]
    fn concurrency_overflow_maps_to_429() {
        let semaphore = Arc::new(Semaphore::new(1));
        let _held = semaphore.clone().try_acquire_owned().unwrap();
        let error = semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| ArturError::TooManyRequests("endpoint concurrency limit reached".into()))
            .unwrap_err();
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
    }

    #[test]
    fn successful_rate_limited_response_has_current_draft_headers() {
        let mut response = StatusCode::OK.into_response();
        add_rate_limit_headers(
            &mut response,
            &rate_limit::RateLimitResult {
                allowed: true,
                remaining: 1,
                retry_after: 42,
            },
            &crate::config::RateLimitConfig {
                store: "rate".into(),
                key: "{{client.ip}}".into(),
                requests: 2,
                window_secs: 60,
            },
        );
        assert_eq!(response.headers()["ratelimit"], "\"2\";r=1;t=42");
        assert_eq!(response.headers()["ratelimit-policy"], "\"2\";q=2;w=60");
    }

    #[test]
    fn rejects_invalid_gateway_protection_configuration() {
        for fragment in [
            "[artur.server.client_ip]\ntrusted_proxy_cidrs=[\"not-a-cidr\"]",
            "[artur.server.client_ip]\nheader=\"x-forwarded-for\"",
            "[artur.endpoints.restrictions]\nallowed_content_types=[\"bad\"]",
            "[artur.endpoints.restrictions]\ntimeout_ms=0",
            "[artur.endpoints.restrictions]\nmax_concurrency=1000001",
            "[artur.endpoints.security.rate_limit]\nstore=\"rate\"\nkey=\"{{client.ip}}\"\nrequests=0\nwindow_secs=60",
            "[artur.endpoints.security.rate_limit]\nstore=\"rate\"\nkey=\"{{client.ip}}\"\nrequests=1\nwindow_secs=0",
        ] {
            let raw = format!(
                "version=1\n[stores.rate]\ndriver=\"sqlite\"\nurl=\"sqlite://rate.db\"\n[[artur.endpoints]]\nname=\"x\"\nmethod=\"POST\"\npath=\"/x\"\naction=\"respond.static\"\n[artur.endpoints.response]\nbody={{}}\n{fragment}"
            );
            let config: AppConfig = toml::from_str(&raw).unwrap();
            assert!(config.validate().is_err(), "{fragment}");
        }
    }

    #[test]
    fn normalizes_colon_params() {
        assert_eq!(normalize_axum_path("/jobs/:job_id"), "/jobs/{job_id}");
        assert_eq!(normalize_axum_path("/space/"), "/space/");
    }
}

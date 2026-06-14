use crate::{
    AppConfig,
    config::{EndpointConfig, SecurityTaskConfig},
    error::{ArturError, Result},
    process::{RequestContext, lookup_template_json_value, render_template, run_task},
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Instant};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct SecurityState {
    failures: Arc<RwLock<BTreeMap<String, FailureRecord>>>,
}

#[derive(Debug, Clone)]
struct FailureRecord {
    first_seen: Instant,
    failures: u32,
    blocked_until: Option<Instant>,
}

pub async fn authorize_endpoint(
    config: Arc<AppConfig>,
    security: SecurityState,
    endpoint: &EndpointConfig,
    request: &RequestContext,
) -> Result<()> {
    check_block(&security, endpoint, request).await?;

    let result = async {
        check_api_key(endpoint, request)?;
        check_task_guard(
            config.clone(),
            endpoint.security.challenge.as_ref(),
            request,
            false,
        )
        .await?;
        check_task_guard(config, endpoint.security.x402.as_ref(), request, true).await?;
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            clear_failure(&security, endpoint, request).await;
            Ok(())
        }
        Err(err) => {
            record_failure(&security, endpoint, request).await;
            Err(err)
        }
    }
}

fn check_api_key(endpoint: &EndpointConfig, request: &RequestContext) -> Result<()> {
    let Some(api_key) = &endpoint.security.api_key else {
        return Ok(());
    };
    let header_name = api_key.header.to_ascii_lowercase();
    let expected = render_template(&api_key.value, request)?;
    let actual = request
        .headers
        .get(&header_name)
        .cloned()
        .unwrap_or_default();
    let expected = match &api_key.scheme {
        Some(scheme) if !scheme.trim().is_empty() => format!("{} {}", scheme.trim(), expected),
        _ => expected,
    };
    if !constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
        return Err(ArturError::Forbidden(format!(
            "endpoint {} rejected request: invalid api key",
            endpoint.name
        )));
    }
    Ok(())
}

async fn check_task_guard(
    config: Arc<AppConfig>,
    guard: Option<&SecurityTaskConfig>,
    request: &RequestContext,
    payment: bool,
) -> Result<()> {
    let Some(guard) = guard else {
        return Ok(());
    };
    let task = config
        .task_by_name(&guard.task)
        .cloned()
        .ok_or_else(|| ArturError::Config(format!("unknown security task {}", guard.task)))?;
    let output = run_task(&task, request).await?;
    if output.ok && guard_output_allows(guard, output.json.as_ref(), payment) {
        return Ok(());
    }
    if payment {
        Err(ArturError::PaymentRequired(format!(
            "x402 payment verification failed for task {}",
            guard.task
        )))
    } else {
        Err(ArturError::Forbidden(format!(
            "challenge verification failed for task {}",
            guard.task
        )))
    }
}

fn guard_output_allows(guard: &SecurityTaskConfig, json: Option<&Value>, payment: bool) -> bool {
    let Some(json) = json else {
        return false;
    };
    if let Some(path) = &guard.success_path {
        let request = RequestContext {
            method: String::new(),
            uri: String::new(),
            path: String::new(),
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body: String::new(),
            body_json: None,
            steps: BTreeMap::from([("guard".to_string(), json.clone())]),
        };
        return lookup_template_json_value(&format!("steps.guard.{path}"), &request)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    }
    json.get("ok")
        .and_then(Value::as_bool)
        .or_else(|| json.get("allowed").and_then(Value::as_bool))
        .or_else(|| json.get("verified").and_then(Value::as_bool))
        .or_else(|| {
            if payment {
                json.get("paid").and_then(Value::as_bool)
            } else {
                None
            }
        })
        .unwrap_or(false)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for index in 0..max_len {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(a ^ b);
    }
    diff == 0
}

async fn check_block(
    security: &SecurityState,
    endpoint: &EndpointConfig,
    request: &RequestContext,
) -> Result<()> {
    let Some(block) = &endpoint.security.failure_block else {
        return Ok(());
    };
    let key = failure_key(endpoint, request)?;
    let now = Instant::now();
    let failures = security.failures.read().await;
    if let Some(record) = failures.get(&key)
        && let Some(blocked_until) = record.blocked_until
        && now < blocked_until
    {
        return Err(ArturError::TooManyRequests(format!(
            "endpoint {} temporarily blocked this key after {} failed requests",
            endpoint.name, block.max_failures
        )));
    }
    Ok(())
}

async fn record_failure(
    security: &SecurityState,
    endpoint: &EndpointConfig,
    request: &RequestContext,
) {
    let Some(block) = &endpoint.security.failure_block else {
        return;
    };
    let Ok(key) = failure_key(endpoint, request) else {
        return;
    };
    let now = Instant::now();
    let window = std::time::Duration::from_secs(block.window_secs);
    let mut failures = security.failures.write().await;
    let record = failures.entry(key).or_insert(FailureRecord {
        first_seen: now,
        failures: 0,
        blocked_until: None,
    });
    if now.duration_since(record.first_seen) > window {
        record.first_seen = now;
        record.failures = 0;
        record.blocked_until = None;
    }
    record.failures += 1;
    if record.failures >= block.max_failures {
        record.blocked_until = Some(now + std::time::Duration::from_secs(block.block_secs));
    }
}

async fn clear_failure(
    security: &SecurityState,
    endpoint: &EndpointConfig,
    request: &RequestContext,
) {
    if endpoint.security.failure_block.is_none() {
        return;
    }
    if let Ok(key) = failure_key(endpoint, request) {
        security.failures.write().await.remove(&key);
    }
}

fn failure_key(endpoint: &EndpointConfig, request: &RequestContext) -> Result<String> {
    let Some(block) = &endpoint.security.failure_block else {
        return Ok(String::new());
    };
    let rendered = render_template(&block.key, request)?;
    if rendered.trim().is_empty() {
        Ok(format!("{}:anonymous", endpoint.name))
    } else {
        Ok(format!("{}:{rendered}", endpoint.name))
    }
}

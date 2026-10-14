//! Shared OpenRouter HTTP plumbing: one pooled client, error-body parsing,
//! and automatic retries of transient failures.
//!
//! Retries are invisible to the user; every failed attempt is logged.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

const API_BASE: &str = "https://openrouter.ai/api/v1";
const NO_KEY_MESSAGE: &str = "No OpenRouter API key is set. Add one in Settings.";
/// Retries after the first attempt, so up to 6 attempts in total.
const MAX_RETRIES: u32 = 5;
/// Delay before the first retry; doubles each time (0.5, 1, 2, 4, 8 s).
const BASE_DELAY: Duration = Duration::from_millis(500);
/// Upper bound for any single wait, including a server-supplied Retry-After.
const MAX_DELAY: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
}

enum Failure {
    /// Worth trying again: network trouble, rate limits, server errors.
    Transient {
        message: String,
        retry_after: Option<Duration>,
    },
    /// Retrying cannot help: bad key, no credits, malformed request.
    Permanent(String),
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .expect("could not initialise the HTTP client")
    })
}

/// POST a JSON body to `path` under the OpenRouter API, retrying transient
/// failures. `what` names the call in log lines. Returns the response body.
pub async fn post_json(
    what: &str,
    path: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    retrying(what, MAX_RETRIES, BASE_DELAY, || attempt(path, body, timeout)).await
}

async fn attempt(
    path: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<Vec<u8>, Failure> {
    let transient = |message: String| Failure::Transient { message, retry_after: None };
    let Some(key) = crate::apikey::current() else {
        return Err(Failure::Permanent(NO_KEY_MESSAGE.to_string()));
    };

    let response = client()
        .post(format!("{API_BASE}{path}"))
        .bearer_auth(key)
        .timeout(timeout)
        .json(body)
        .send()
        .await
        .map_err(|e| transient(format!("network error: {e}")))?;

    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let bytes = response
        .bytes()
        .await
        .map_err(|e| transient(format!("network error reading response: {e}")))?;

    if status.is_success() {
        return Ok(bytes.to_vec());
    }

    let detail = serde_json::from_slice::<ErrorEnvelope>(&bytes)
        .map(|e| e.error.message)
        .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
    let message = format!("OpenRouter returned {status}: {detail}");
    if is_transient_status(status.as_u16()) {
        Err(Failure::Transient { message, retry_after })
    } else {
        Err(Failure::Permanent(message))
    }
}

/// Outcome of asking OpenRouter about the saved key.
#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KeyCheck {
    /// No key has been saved yet.
    NoKey,
    /// OpenRouter accepted the key. Amounts are USD; `limit_remaining` is
    /// absent when the key has no spending limit.
    Valid { usage: Option<f64>, limit_remaining: Option<f64> },
    /// OpenRouter rejected the key.
    Rejected,
    /// The check itself failed, so nothing is known about the key.
    Unreachable { message: String },
}

#[derive(Deserialize)]
struct KeyInfoEnvelope {
    data: KeyInfo,
}

#[derive(Deserialize)]
struct KeyInfo {
    usage: Option<f64>,
    limit_remaining: Option<f64>,
}

/// A single, unretried request, since a person is waiting on the answer.
pub async fn check_key() -> KeyCheck {
    let Some(key) = crate::apikey::current() else {
        return KeyCheck::NoKey;
    };
    let response = client()
        .get(format!("{API_BASE}/key"))
        .bearer_auth(key)
        .timeout(Duration::from_secs(15))
        .send()
        .await;
    let response = match response {
        Ok(r) => r,
        Err(e) => return KeyCheck::Unreachable { message: format!("network error: {e}") },
    };
    match response.status().as_u16() {
        200 => match response.json::<KeyInfoEnvelope>().await {
            Ok(info) => KeyCheck::Valid {
                usage: info.data.usage,
                limit_remaining: info.data.limit_remaining,
            },
            // Accepted, just not in the shape expected.
            Err(_) => KeyCheck::Valid { usage: None, limit_remaining: None },
        },
        401 | 403 => KeyCheck::Rejected,
        other => KeyCheck::Unreachable { message: format!("OpenRouter returned HTTP {other}") },
    }
}

fn is_transient_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500..=599)
}

fn backoff(base: Duration, attempt: u32) -> Duration {
    base * 2u32.saturating_pow(attempt.saturating_sub(1))
}

async fn retrying<T, F, Fut>(
    what: &str,
    max_retries: u32,
    base_delay: Duration,
    mut op: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Failure>>,
{
    let total = max_retries + 1;
    let mut attempt = 1;
    loop {
        match op().await {
            Ok(value) => {
                if attempt > 1 {
                    log::info!("{what} succeeded on attempt {attempt} of {total}");
                }
                return Ok(value);
            }
            Err(Failure::Permanent(message)) => {
                log::error!("{what} failed, not retryable: {message}");
                return Err(message);
            }
            Err(Failure::Transient { message, retry_after }) => {
                if attempt >= total {
                    log::error!("{what} failed after {total} attempts: {message}");
                    return Err(format!("{message} (gave up after {total} attempts)"));
                }
                let delay = retry_after
                    .unwrap_or_else(|| backoff(base_delay, attempt))
                    .min(MAX_DELAY);
                log::warn!(
                    "{what} attempt {attempt} of {total} failed: {message}; retrying in {} ms",
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const TINY: Duration = Duration::from_millis(1);

    fn run<T>(fut: impl Future<Output = T>) -> T {
        tauri::async_runtime::block_on(fut)
    }

    fn transient() -> Failure {
        Failure::Transient { message: "boom".into(), retry_after: None }
    }

    #[test]
    fn classifies_statuses() {
        for retryable in [408, 425, 429, 500, 502, 503, 599] {
            assert!(is_transient_status(retryable), "{retryable} should retry");
        }
        for fatal in [400, 401, 402, 403, 404, 422] {
            assert!(!is_transient_status(fatal), "{fatal} should not retry");
        }
    }

    #[test]
    fn backoff_doubles() {
        let base = Duration::from_millis(500);
        let waits: Vec<u128> = (1..=5).map(|a| backoff(base, a).as_millis()).collect();
        assert_eq!(waits, vec![500, 1000, 2000, 4000, 8000]);
    }

    #[test]
    fn retries_transient_failures_then_succeeds() {
        let calls = Cell::new(0);
        let result = run(retrying("test", 5, TINY, || {
            let n = calls.get() + 1;
            calls.set(n);
            async move { if n <= 2 { Err(transient()) } else { Ok(n) } }
        }));
        assert_eq!(result, Ok(3));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn does_not_retry_permanent_failures() {
        let calls = Cell::new(0);
        let result: Result<u32, String> = run(retrying("test", 5, TINY, || {
            calls.set(calls.get() + 1);
            async { Err(Failure::Permanent("bad key".into())) }
        }));
        assert_eq!(result, Err("bad key".to_string()));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn gives_up_after_five_retries() {
        let calls = Cell::new(0);
        let result: Result<u32, String> = run(retrying("test", 5, TINY, || {
            calls.set(calls.get() + 1);
            async { Err(transient()) }
        }));
        assert_eq!(calls.get(), 6, "1 attempt + 5 retries");
        assert!(result.unwrap_err().contains("gave up after 6 attempts"));
    }
}

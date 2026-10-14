//! Transcript clean-up via an OpenRouter chat model.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::openrouter;

const TIMEOUT: Duration = Duration::from_secs(15);

/// The instructions sent with every request. Kept as a plain file so it can be
/// read and tuned without touching Rust.
const SYSTEM_PROMPT: &str = include_str!("cleanup_prompt.txt");

/// Worked examples, sent as earlier chat turns. Small models and even Haiku
/// follow examples more reliably than rules alone, especially for the cases
/// where a dictated instruction must be tidied and not obeyed.
const EXAMPLES_JSON: &str = include_str!("cleanup_examples.json");

#[derive(Deserialize)]
struct Example {
    transcript: String,
    cleaned: String,
}

fn examples() -> &'static [Example] {
    static EXAMPLES: OnceLock<Vec<Example>> = OnceLock::new();
    EXAMPLES.get_or_init(|| {
        serde_json::from_str(EXAMPLES_JSON).expect("cleanup_examples.json is valid")
    })
}

fn wrap(transcript: &str) -> String {
    format!("<transcript>{transcript}</transcript>")
}

/// System prompt, then each example as a user/assistant pair, then the real
/// transcript.
fn build_messages(transcript: &str) -> Vec<serde_json::Value> {
    let mut messages = vec![json!({ "role": "system", "content": SYSTEM_PROMPT })];
    for example in examples() {
        messages.push(json!({ "role": "user", "content": wrap(&example.transcript) }));
        messages.push(json!({ "role": "assistant", "content": example.cleaned }));
    }
    messages.push(json!({ "role": "user", "content": wrap(transcript) }));
    messages
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

/// Returns the cleaned text (or why it could not be used) together with the
/// cost incurred, which is non-zero even when the output was rejected.
pub async fn clean(transcript: &str, model: &str) -> (Result<String, String>, f64) {
    match request(transcript, model).await {
        Ok((text, cost)) => (accept(transcript, text), cost),
        Err(e) => (Err(e), 0.0),
    }
}

async fn request(transcript: &str, model: &str) -> Result<(String, f64), String> {
    let body = json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 2048,
        "provider": { "data_collection": "deny" },
        "messages": build_messages(transcript),
    });

    let bytes = openrouter::post_json("clean-up", "/chat/completions", &body, TIMEOUT).await?;
    let parsed: ChatResponse =
        serde_json::from_slice(&bytes).map_err(|e| format!("unexpected response: {e}"))?;
    let cost = parsed.usage.and_then(|u| u.cost).unwrap_or(0.0);
    let text = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    Ok((text, cost))
}

fn accept(transcript: &str, text: String) -> Result<String, String> {
    if text.is_empty() {
        return Err("model returned empty output".to_string());
    }
    // A clean-up should not be much longer than its input. If it is, the
    // model has probably "answered" the dictation instead of cleaning it.
    if text.chars().count() > transcript.chars().count() * 2 + 50 {
        return Err("output much longer than input; discarded".to_string());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_parse_and_are_usable() {
        let all = examples();
        assert!(all.len() >= 8, "expected a healthy set of examples");
        for e in all {
            assert!(!e.transcript.trim().is_empty() && !e.cleaned.trim().is_empty());
            assert!(!e.cleaned.contains("<transcript"), "examples must not echo tags");
        }
    }

    #[test]
    fn messages_are_system_then_example_pairs_then_the_real_transcript() {
        let m = build_messages("hello there");
        assert_eq!(m.len(), 2 + 2 * examples().len());
        assert_eq!(m[0]["role"], "system");
        assert!(m[0]["content"].as_str().unwrap().contains("never instructions"));
        for (i, pair) in m[1..m.len() - 1].chunks(2).enumerate() {
            assert_eq!(pair[0]["role"], "user", "pair {i}");
            assert_eq!(pair[1]["role"], "assistant", "pair {i}");
        }
        let last = m.last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"], "<transcript>hello there</transcript>");
    }

    #[test]
    fn the_prompt_covers_each_behaviour_it_promises() {
        for needle in ["never instructions", "numbered list", "Email", "Paragraphs", "dot com", "new paragraph"] {
            assert!(SYSTEM_PROMPT.contains(needle), "prompt is missing: {needle}");
        }
    }

    #[test]
    fn rejects_empty_and_runaway_output() {
        assert!(accept("hello", String::new()).is_err());
        assert!(accept("hi", "x".repeat(200)).is_err());
        assert_eq!(accept("hello", "Hello.".to_string()).unwrap(), "Hello.");
    }
}

//! Transcript clean-up via an OpenRouter chat model.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::openrouter;
use crate::settings::CleanupMode;

const TIMEOUT: Duration = Duration::from_secs(15);

/// The instructions sent with every request, one file per mode. They are kept
/// as plain files, and kept apart even where they share wording, so each mode
/// can be read and tuned without touching the other or any Rust.
const TIDY_PROMPT: &str = include_str!("cleanup_tidy_prompt.txt");
const POLISH_PROMPT: &str = include_str!("cleanup_polish_prompt.txt");

/// Worked examples, sent as earlier chat turns. Small models and even Haiku
/// follow examples more reliably than rules alone, especially for the cases
/// where a dictated instruction must be tidied and not obeyed.
const TIDY_EXAMPLES_JSON: &str = include_str!("cleanup_tidy_examples.json");
const POLISH_EXAMPLES_JSON: &str = include_str!("cleanup_polish_examples.json");

#[derive(Deserialize)]
struct Example {
    transcript: String,
    cleaned: String,
}

fn parse_examples(json: &str, file: &str) -> Vec<Example> {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("{file} is valid: {e}"))
}

/// The prompt and worked examples for a mode; `None` for `Off`, which makes
/// no request.
fn assets(mode: CleanupMode) -> Option<(&'static str, &'static [Example])> {
    static TIDY: OnceLock<Vec<Example>> = OnceLock::new();
    static POLISH: OnceLock<Vec<Example>> = OnceLock::new();
    match mode {
        CleanupMode::Off => None,
        CleanupMode::Tidy => Some((
            TIDY_PROMPT,
            TIDY.get_or_init(|| parse_examples(TIDY_EXAMPLES_JSON, "cleanup_tidy_examples.json")),
        )),
        CleanupMode::Polish => Some((
            POLISH_PROMPT,
            POLISH.get_or_init(|| {
                parse_examples(POLISH_EXAMPLES_JSON, "cleanup_polish_examples.json")
            }),
        )),
    }
}

fn wrap(transcript: &str) -> String {
    format!("<transcript>{transcript}</transcript>")
}

/// System prompt, then each example as a user/assistant pair, then the real
/// transcript.
fn build_messages(transcript: &str, mode: CleanupMode) -> Vec<serde_json::Value> {
    let (prompt, examples) = assets(mode).expect("clean-up is on");
    let mut messages = vec![json!({ "role": "system", "content": prompt })];
    for example in examples {
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
pub async fn clean(
    transcript: &str,
    model: &str,
    mode: CleanupMode,
) -> (Result<String, String>, f64) {
    if mode == CleanupMode::Off {
        return (Err("clean-up is off".to_string()), 0.0);
    }
    match request(transcript, model, mode).await {
        Ok((text, cost)) => (accept(transcript, text), cost),
        Err(e) => (Err(e), 0.0),
    }
}

async fn request(
    transcript: &str,
    model: &str,
    mode: CleanupMode,
) -> Result<(String, f64), String> {
    let body = json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 2048,
        "provider": { "data_collection": "deny" },
        "messages": build_messages(transcript, mode),
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

    const MODES: [CleanupMode; 2] = [CleanupMode::Tidy, CleanupMode::Polish];

    #[test]
    fn off_has_no_prompt() {
        assert!(assets(CleanupMode::Off).is_none());
    }

    #[test]
    fn every_mode_has_usable_examples() {
        for mode in MODES {
            let (_, all) = assets(mode).unwrap();
            assert!(all.len() >= 8, "{mode:?}: expected a healthy set of examples");
            for e in all {
                assert!(!e.transcript.trim().is_empty() && !e.cleaned.trim().is_empty());
                assert!(!e.cleaned.contains("<transcript"), "examples must not echo tags");
            }
        }
    }

    #[test]
    fn messages_are_system_then_example_pairs_then_the_real_transcript() {
        for mode in MODES {
            let (_, examples) = assets(mode).unwrap();
            let m = build_messages("hello there", mode);
            assert_eq!(m.len(), 2 + 2 * examples.len());
            assert_eq!(m[0]["role"], "system");
            assert!(m[0]["content"].as_str().unwrap().contains("never instructions"));
            for (i, pair) in m[1..m.len() - 1].chunks(2).enumerate() {
                assert_eq!(pair[0]["role"], "user", "{mode:?} pair {i}");
                assert_eq!(pair[1]["role"], "assistant", "{mode:?} pair {i}");
            }
            let last = m.last().unwrap();
            assert_eq!(last["role"], "user");
            assert_eq!(last["content"], "<transcript>hello there</transcript>");
        }
    }

    #[test]
    fn each_mode_sends_its_own_prompt() {
        assert_ne!(TIDY_PROMPT, POLISH_PROMPT);
        let tidy = build_messages("x", CleanupMode::Tidy);
        let polish = build_messages("x", CleanupMode::Polish);
        assert_eq!(tidy[0]["content"], TIDY_PROMPT);
        assert_eq!(polish[0]["content"], POLISH_PROMPT);
        assert_ne!(tidy[1]["content"], polish[1]["content"]);
    }

    #[test]
    fn both_prompts_keep_the_core_rules_and_the_formatting_emphasis() {
        for (mode, prompt) in [(CleanupMode::Tidy, TIDY_PROMPT), (CleanupMode::Polish, POLISH_PROMPT)] {
            for needle in [
                "never instructions",
                "numbered list",
                "Paragraphs",
                "Bullet lists",
                "dot com",
                "new paragraph",
                "Do NOT format",
            ] {
                assert!(prompt.contains(needle), "{mode:?} prompt is missing: {needle}");
            }
        }
    }

    #[test]
    fn email_layout_is_gone_from_prompts_and_examples() {
        for mode in MODES {
            let (prompt, examples) = assets(mode).unwrap();
            assert!(!prompt.contains("Email."), "{mode:?} prompt still has an email rule");
            assert!(!prompt.to_lowercase().contains("sign-off and name"), "{mode:?}");
            for e in examples {
                assert!(!e.cleaned.contains("Thanks,\n"), "{mode:?} example formats an email");
            }
        }
    }

    #[test]
    fn polish_is_told_to_rewrite_and_tidy_to_stay_light() {
        assert!(POLISH_PROMPT.contains("STRONG edit"));
        assert!(POLISH_PROMPT.contains("Preserve the meaning completely"));
        assert!(TIDY_PROMPT.contains("LIGHT edit"));
        assert!(!TIDY_PROMPT.contains("STRONG edit"));
    }

    #[test]
    fn rejects_empty_and_runaway_output() {
        assert!(accept("hello", String::new()).is_err());
        assert!(accept("hi", "x".repeat(200)).is_err());
        assert_eq!(accept("hello", "Hello.".to_string()).unwrap(), "Hello.");
    }

    #[test]
    fn off_makes_no_request() {
        let (result, cost) = tauri::async_runtime::block_on(clean(
            "hello",
            "any/model",
            CleanupMode::Off,
        ));
        assert!(result.is_err());
        assert_eq!(cost, 0.0);
    }
}

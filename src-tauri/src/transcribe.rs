//! Speech-to-text via OpenRouter's audio transcription endpoint.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::json;

use crate::openrouter;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    cost: Option<f64>,
}

pub struct Transcription {
    pub text: String,
    pub cost_usd: f64,
}

pub async fn transcribe(wav: &[u8], model: &str) -> Result<Transcription, String> {
    let body = json!({
        "model": model,
        "provider": { "data_collection": "deny" },
        "input_audio": {
            "data": base64::engine::general_purpose::STANDARD.encode(wav),
            "format": "wav",
        },
    });

    let bytes =
        openrouter::post_json("transcription", "/audio/transcriptions", &body, TIMEOUT).await?;
    let parsed: TranscriptionResponse =
        serde_json::from_slice(&bytes).map_err(|e| format!("unexpected response: {e}"))?;
    Ok(Transcription {
        text: parsed.text.trim().to_string(),
        cost_usd: parsed.usage.and_then(|u| u.cost).unwrap_or(0.0),
    })
}

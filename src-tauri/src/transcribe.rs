//! Speech-to-text via OpenRouter's audio transcription endpoint.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::json;

const ENDPOINT: &str = "https://openrouter.ai/api/v1/audio/transcriptions";
const MODEL: &str = "openai/whisper-1";

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

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
}

pub async fn transcribe(wav: &[u8]) -> Result<Transcription, String> {
    let body = json!({
        "model": MODEL,
        "input_audio": {
            "data": base64::engine::general_purpose::STANDARD.encode(wav),
            "format": "wav",
        },
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .post(ENDPOINT)
        .bearer_auth(env!("OPENROUTER_API_KEY"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;

    let status = response.status();
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        let detail = serde_json::from_slice::<ErrorEnvelope>(&bytes)
            .map(|e| e.error.message)
            .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
        return Err(format!("OpenRouter returned {status}: {detail}"));
    }

    let parsed: TranscriptionResponse =
        serde_json::from_slice(&bytes).map_err(|e| format!("unexpected response: {e}"))?;
    Ok(Transcription {
        text: parsed.text.trim().to_string(),
        cost_usd: parsed.usage.and_then(|u| u.cost).unwrap_or(0.0),
    })
}

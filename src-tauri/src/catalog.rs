//! The OpenRouter models offered in the settings drop-downs. These were
//! tried against the app's real request and prompt; anything else can still
//! be entered by hand under "Other".

use serde::Serialize;

pub struct CloudModel {
    pub id: &'static str,
    pub label: &'static str,
}

pub const TRANSCRIPTION_MODELS: &[CloudModel] = &[
    CloudModel { id: "openai/whisper-1", label: "OpenAI Whisper 1" },
    CloudModel { id: "openai/whisper-large-v3-turbo", label: "OpenAI Whisper Large v3 Turbo" },
    CloudModel { id: "deepgram/nova-3", label: "Deepgram Nova 3" },
    CloudModel { id: "mistralai/voxtral-mini-transcribe", label: "Mistral Voxtral Mini Transcribe" },
    CloudModel { id: "openai/gpt-4o-mini-transcribe", label: "OpenAI GPT-4o Mini Transcribe" },
    CloudModel { id: "elevenlabs/scribe-v2", label: "ElevenLabs Scribe v2" },
];

pub const CLEANUP_MODELS: &[CloudModel] = &[
    CloudModel { id: "anthropic/claude-haiku-5.5", label: "Anthropic Claude Haiku 5.5" },
    CloudModel { id: "openai/gpt-6-luna", label: "OpenAI GPT-6 Luna" },
    CloudModel { id: "deepseek/deepseek-v4-flash", label: "DeepSeek V4 Flash" },
];

#[derive(Serialize)]
pub struct ModelOption {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Serialize)]
pub struct CloudCatalog {
    pub transcription: Vec<ModelOption>,
    pub cleanup: Vec<ModelOption>,
}

fn options(models: &[CloudModel]) -> Vec<ModelOption> {
    models
        .iter()
        .map(|m| ModelOption { id: m.id, label: m.label })
        .collect()
}

pub fn cloud_catalog() -> CloudCatalog {
    CloudCatalog {
        transcription: options(TRANSCRIPTION_MODELS),
        cleanup: options(CLEANUP_MODELS),
    }
}

/// Tidy a user-supplied model id: surrounding whitespace is dropped, an empty
/// value means "the default", and an id containing whitespace is rejected as
/// a typo rather than sent to OpenRouter.
pub fn normalize_model_id(raw: &str, default: &str) -> Result<String, String> {
    let id = raw.trim();
    if id.is_empty() {
        return Ok(default.to_string());
    }
    if id.chars().any(char::is_whitespace) {
        return Err("model ids cannot contain spaces".to_string());
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{DEFAULT_CLEANUP_MODEL, DEFAULT_CLOUD_TRANSCRIPTION_MODEL};

    #[test]
    fn defaults_are_in_their_lists() {
        assert!(TRANSCRIPTION_MODELS.iter().any(|m| m.id == DEFAULT_CLOUD_TRANSCRIPTION_MODEL));
        assert!(CLEANUP_MODELS.iter().any(|m| m.id == DEFAULT_CLEANUP_MODEL));
    }

    #[test]
    fn ids_are_unique_and_look_like_openrouter_ids() {
        for list in [TRANSCRIPTION_MODELS, CLEANUP_MODELS] {
            let mut seen = std::collections::HashSet::new();
            for m in list {
                assert!(seen.insert(m.id), "duplicate id {}", m.id);
                assert!(m.id.contains('/') && !m.id.contains(char::is_whitespace), "{}", m.id);
                assert!(!m.label.is_empty());
            }
        }
    }

    #[test]
    fn normalizing_model_ids() {
        assert_eq!(normalize_model_id("  vendor/model  ", "d/d").unwrap(), "vendor/model");
        assert_eq!(normalize_model_id("   ", "d/d").unwrap(), "d/d");
        assert_eq!(normalize_model_id("", "d/d").unwrap(), "d/d");
        assert!(normalize_model_id("vendor/my model", "d/d").is_err());
        assert!(normalize_model_id("vendor/model\tx", "d/d").is_err());
    }
}

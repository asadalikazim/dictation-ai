//! Local speech-to-text with whisper.cpp (via whisper-rs, Metal on Apple
//! Silicon). The models ship inside the app bundle; see
//! scripts/fetch-models.sh for how they get there.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::transcribe::Transcription;

pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub file: &'static str,
    /// Approximate size, for display only.
    pub size_mb: u32,
}

pub const MODELS: &[ModelInfo] = &[
    ModelInfo { id: "base.en", label: "Base, English", file: "ggml-base.en.bin", size_mb: 148 },
    ModelInfo { id: "small.en", label: "Small, English", file: "ggml-small.en.bin", size_mb: 488 },
    ModelInfo {
        id: "large-v3-turbo-q5_0",
        label: "Large v3 Turbo, quantized",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size_mb: 574,
    },
];

#[derive(Serialize)]
pub struct ModelView {
    pub id: &'static str,
    pub label: &'static str,
    pub size_mb: u32,
}

pub fn views() -> Vec<ModelView> {
    MODELS
        .iter()
        .map(|m| ModelView { id: m.id, label: m.label, size_mb: m.size_mb })
        .collect()
}

pub fn find(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

fn model_path(app: &AppHandle, info: &ModelInfo) -> Result<PathBuf, String> {
    // Development runs read straight from the source tree; the bundled copy
    // (under resources/ in the .app) is what release builds use.
    if cfg!(debug_assertions) {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/models")
            .join(info.file);
        if dev.exists() {
            return Ok(dev);
        }
    }
    let bundled = app
        .path()
        .resolve(format!("resources/models/{}", info.file), BaseDirectory::Resource)
        .map_err(|e| e.to_string())?;
    if bundled.exists() {
        Ok(bundled)
    } else {
        Err(format!(
            "model file {} is missing; run scripts/fetch-models.sh and rebuild",
            info.file
        ))
    }
}

struct Loaded {
    id: String,
    context: Arc<WhisperContext>,
}

/// Only the model in use stays in memory; switching drops the previous one.
static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

fn context_for(app: &AppHandle, id: &str) -> Result<Arc<WhisperContext>, String> {
    let mut loaded = LOADED.lock().unwrap();
    if let Some(l) = loaded.as_ref() {
        if l.id == id {
            return Ok(Arc::clone(&l.context));
        }
    }
    let info = find(id).ok_or_else(|| format!("unknown local model {id:?}"))?;
    let path = model_path(app, info)?;

    // Free the old model before allocating the new one.
    *loaded = None;
    let started = Instant::now();
    let context = WhisperContext::new_with_params(&path, WhisperContextParameters::default())
        .map_err(|e| format!("could not load {}: {e}", info.file))?;
    log::info!("loaded local model {id} in {} ms", started.elapsed().as_millis());

    let context = Arc::new(context);
    *loaded = Some(Loaded { id: id.to_string(), context: Arc::clone(&context) });
    Ok(context)
}

/// Load a model in the background so the first dictation does not pay for it.
pub fn preload(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = context_for(&app, &id) {
            log::error!("could not preload local model {id}: {e}");
        }
    });
}

/// Runs inference on a blocking thread. `samples` are 16 kHz mono f32.
pub async fn transcribe(
    app: &AppHandle,
    model_id: &str,
    samples: Vec<f32>,
) -> Result<Transcription, String> {
    let app = app.clone();
    let id = model_id.to_string();
    tauri::async_runtime::spawn_blocking(move || run(&app, &id, &samples))
        .await
        .map_err(|e| format!("transcription task failed: {e}"))?
}

fn run(app: &AppHandle, id: &str, samples: &[f32]) -> Result<Transcription, String> {
    let context = context_for(app, id)?;
    let started = Instant::now();
    let text = infer(&context, samples)?;
    log::info!(
        "local inference with {id} took {} ms for {:.1}s of audio",
        started.elapsed().as_millis(),
        samples.len() as f32 / 16_000.0
    );
    Ok(Transcription { text, cost_usd: 0.0 })
}

fn infer(context: &WhisperContext, samples: &[f32]) -> Result<String, String> {
    let mut state = context
        .create_state()
        .map_err(|e| format!("could not create whisper state: {e}"))?;

    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(threads as i32);
    params.set_language(Some("en"));
    params.set_translate(false);
    params.set_no_context(true);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);

    state
        .full(params, samples)
        .map_err(|e| format!("local transcription failed: {e}"))?;

    let mut text = String::new();
    for segment in state.as_iter() {
        let piece = segment
            .to_str_lossy()
            .map_err(|e| format!("could not read segment: {e}"))?;
        text.push_str(&piece);
    }
    Ok(text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rss_mb() -> u64 {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().unwrap_or(0) / 1024
    }

    fn read_wav(path: &str) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).expect("wav");
        assert_eq!(reader.spec().sample_rate, 16_000, "benchmark wav must be 16 kHz");
        reader
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / f32::from(i16::MAX))
            .collect()
    }

    /// Prints load time, inference time and process memory per bundled model.
    /// Run: BENCH_WAVS=a.wav,b.wav cargo test --lib bench_models -- --ignored --nocapture
    #[test]
    #[ignore = "benchmark; needs the models and BENCH_WAVS"]
    fn bench_models() {
        let wavs = std::env::var("BENCH_WAVS").expect("set BENCH_WAVS=a.wav,b.wav");
        let clips: Vec<(String, Vec<f32>)> =
            wavs.split(',').map(|p| (p.to_string(), read_wav(p))).collect();
        println!("baseline process memory: {} MB", rss_mb());

        for info in MODELS {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/models")
                .join(info.file);
            let t = Instant::now();
            let ctx = WhisperContext::new_with_params(&path, WhisperContextParameters::default())
                .expect("load");
            println!("\n== {} ==\nload: {} ms", info.id, t.elapsed().as_millis());

            for (name, samples) in &clips {
                let secs = samples.len() as f32 / 16_000.0;
                // First call warms up GPU kernels; report it separately.
                let mut times = Vec::new();
                let mut text = String::new();
                for _ in 0..3 {
                    let t = Instant::now();
                    text = infer(&ctx, samples).expect("infer");
                    times.push(t.elapsed().as_millis());
                }
                println!(
                    "{:>5.1}s clip ({}): first {} ms, then {} ms / {} ms",
                    secs,
                    name.rsplit('/').next().unwrap_or(name),
                    times[0], times[1], times[2]
                );
                println!("   text: {text}");
            }
            println!("process memory with {} loaded: {} MB", info.id, rss_mb());
            drop(ctx);
            println!("after unload: {} MB", rss_mb());
        }
    }
}

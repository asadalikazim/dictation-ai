use std::path::Path;

fn main() {
    // The OpenRouter key is a build-time constant read from the repo-root .env.
    // It is handed to rustc only, so it exists in the Rust binary and nowhere
    // in the frontend bundle.
    let env_path = Path::new("../.env");
    println!("cargo:rerun-if-changed=../.env");

    let key = dotenvy::from_path_iter(env_path)
        .ok()
        .and_then(|iter| {
            iter.filter_map(Result::ok)
                .find(|(k, _)| k == "OPENROUTER_API_KEY")
                .map(|(_, v)| v)
        })
        .filter(|v| !v.trim().is_empty());

    match key {
        Some(key) => println!("cargo:rustc-env=OPENROUTER_API_KEY={}", key.trim()),
        None => panic!(
            "OPENROUTER_API_KEY is missing. Copy .env.example to .env at the repo root \
             and set OPENROUTER_API_KEY before building."
        ),
    }

    // The Whisper models are bundled into the app but not kept in git.
    // Keep this list in step with MODELS in src/local.rs.
    let models_dir = Path::new("resources/models");
    println!("cargo:rerun-if-changed=resources/models");
    let missing: Vec<&str> = [
        "ggml-base.en.bin",
        "ggml-small.en.bin",
        "ggml-large-v3-turbo-q5_0.bin",
    ]
    .into_iter()
    .filter(|f| !models_dir.join(f).exists())
    .collect();
    if !missing.is_empty() {
        panic!(
            "Whisper model files are missing from src-tauri/resources/models: {}. \
             Run scripts/fetch-models.sh from the repo root first.",
            missing.join(", ")
        );
    }

    tauri_build::build()
}

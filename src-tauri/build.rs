use std::path::Path;

fn main() {
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

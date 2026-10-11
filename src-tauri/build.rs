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

    tauri_build::build()
}

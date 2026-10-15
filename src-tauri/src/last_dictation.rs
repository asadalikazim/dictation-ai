//! The most recent dictation: its transcript, its cleaned text, and which
//! models were used, how long each step took and what each cost. Kept as a
//! JSON file in the app data directory and overwritten on every run, so it
//! holds exactly one dictation.
//!
//! This is the one place the app stores what was said. Logs and the usage
//! tally deliberately never do.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::Engine;

/// Where the clean-up step stands for the dictation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CleanupState {
    /// Clean-up is running; the transcript is already on disk.
    Pending,
    Done,
    /// Clean-up was tried and failed; the raw transcript was pasted.
    Failed,
    /// Clean-up is switched off in Settings.
    Off,
}

/// What is known as soon as transcription has finished.
pub struct Transcribed<'a> {
    pub audio_seconds: f64,
    pub ms: u64,
    pub engine: Engine,
    /// OpenRouter model id (cloud) or bundled model name (local).
    pub model: &'a str,
    pub cost_usd: f64,
    pub text: &'a str,
    /// The clean-up model, or `None` when clean-up is switched off.
    pub cleanup_model: Option<&'a str>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LastDictation {
    /// Unix seconds, when transcription finished.
    pub recorded_at: u64,
    pub audio_seconds: f64,
    pub transcription_ms: u64,
    pub transcription_engine: Engine,
    pub transcription_model: String,
    pub transcription_cost_usd: f64,
    pub transcript: String,
    pub cleanup: CleanupState,
    /// `None` when clean-up was switched off.
    pub cleanup_model: Option<String>,
    /// `None` until clean-up has run.
    pub cleanup_ms: Option<u64>,
    /// `None` until clean-up has run; non-zero even when it failed.
    pub cleanup_cost_usd: Option<f64>,
    /// Present only when `cleanup` is `Done`.
    pub cleaned: Option<String>,
    /// Milliseconds from key release to the text being pasted; `None` until
    /// the paste has succeeded.
    pub total_ms: Option<u64>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct LastDictationStore {
    path: PathBuf,
    data: Mutex<Option<LastDictation>>,
}

impl LastDictationStore {
    /// A missing or unreadable file means there is no last dictation yet.
    pub fn load(path: PathBuf) -> Self {
        // Earlier builds kept this in a text file. It holds dictated text,
        // so do not leave it lying around unused.
        let _ = std::fs::remove_file(path.with_extension("txt"));
        let data = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LastDictation>(&bytes).ok());
        Self { path, data: Mutex::new(data) }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Replace the last dictation as soon as transcription has finished,
    /// before clean-up starts. Clean-up shows as pending, or as off when
    /// there is no clean-up model.
    pub fn begin(&self, t: Transcribed) {
        let cleanup =
            if t.cleanup_model.is_some() { CleanupState::Pending } else { CleanupState::Off };
        let last = LastDictation {
            recorded_at: now_secs(),
            audio_seconds: t.audio_seconds,
            transcription_ms: t.ms,
            transcription_engine: t.engine,
            transcription_model: t.model.to_string(),
            transcription_cost_usd: t.cost_usd,
            transcript: t.text.to_string(),
            cleanup,
            cleanup_model: t.cleanup_model.map(str::to_string),
            cleanup_ms: None,
            cleanup_cost_usd: None,
            cleaned: None,
            total_ms: None,
        };
        let mut data = self.data.lock().unwrap();
        self.persist(&last);
        *data = Some(last);
    }

    /// Record how clean-up ended: the cleaned text, or `None` if it failed.
    pub fn finish_cleanup(&self, cleaned: Option<&str>, cleanup_ms: u64, cost_usd: f64) {
        self.update(|last| {
            last.cleanup =
                if cleaned.is_some() { CleanupState::Done } else { CleanupState::Failed };
            last.cleanup_ms = Some(cleanup_ms);
            last.cleanup_cost_usd = Some(cost_usd);
            last.cleaned = cleaned.map(str::to_string);
        });
    }

    /// Record the time from key release to the paste having succeeded.
    pub fn finish_paste(&self, total_ms: u64) {
        self.update(|last| last.total_ms = Some(total_ms));
    }

    pub fn snapshot(&self) -> Option<LastDictation> {
        self.data.lock().unwrap().clone()
    }

    /// Forget the last dictation and delete its file.
    pub fn clear(&self) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        *data = None;
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    /// Change the current dictation and save it. Does nothing when there is
    /// none, for example because it was cleared while clean-up was running.
    fn update(&self, change: impl FnOnce(&mut LastDictation)) {
        let mut data = self.data.lock().unwrap();
        if let Some(last) = data.as_mut() {
            change(last);
            self.persist(last);
        }
    }

    fn persist(&self, last: &LastDictation) {
        if let Err(e) = self.save(last) {
            log::warn!("could not save the last dictation: {e}");
        }
    }

    /// Write to a temp file then rename, so a crash cannot leave a torn file.
    fn save(&self, last: &LastDictation) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(last).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("dictation-ai-last-{name}-{}.json", std::process::id()))
    }

    fn transcribed<'a>(text: &'a str, cleanup_model: Option<&'a str>) -> Transcribed<'a> {
        Transcribed {
            audio_seconds: 3.0,
            ms: 900,
            engine: Engine::Local,
            model: "large-v3-turbo-q5_0",
            cost_usd: 0.0,
            text,
            cleanup_model,
        }
    }

    #[test]
    fn garbage_is_not_a_last_dictation() {
        let path = temp_path("garbage");
        std::fs::write(&path, b"nope").unwrap();
        assert!(LastDictationStore::load(path.clone()).snapshot().is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn written_in_steps_and_each_step_survives_a_reload() {
        let path = temp_path("steps");
        let _ = std::fs::remove_file(&path);
        let store = LastDictationStore::load(path.clone());
        assert!(store.snapshot().is_none());

        store.begin(transcribed("raw words\n\nsecond paragraph", Some("some/model")));
        let early = LastDictationStore::load(path.clone()).snapshot().unwrap();
        assert_eq!(early.cleanup, CleanupState::Pending);
        assert_eq!(early.transcript, "raw words\n\nsecond paragraph");
        assert_eq!(early.transcription_engine, Engine::Local);
        assert_eq!(early.transcription_model, "large-v3-turbo-q5_0");
        assert_eq!(early.cleanup_model.as_deref(), Some("some/model"));
        assert!(early.cleanup_cost_usd.is_none() && early.total_ms.is_none());
        assert!(early.recorded_at > 0);

        store.finish_cleanup(Some("Raw words."), 400, 0.0003);
        let done = LastDictationStore::load(path.clone()).snapshot().unwrap();
        assert_eq!(done.cleanup, CleanupState::Done);
        assert_eq!(done.cleaned.as_deref(), Some("Raw words."));
        assert_eq!(done.cleanup_ms, Some(400));
        assert_eq!(done.cleanup_cost_usd, Some(0.0003));

        store.finish_paste(1500);
        let pasted = LastDictationStore::load(path.clone()).snapshot().unwrap();
        assert_eq!(pasted.total_ms, Some(1500));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn failed_clean_up_keeps_its_cost_and_no_cleaned_text() {
        let path = temp_path("failed");
        let store = LastDictationStore::load(path.clone());
        store.begin(transcribed("hello", Some("some/model")));
        store.finish_cleanup(None, 15_000, 0.0001);
        let last = store.snapshot().unwrap();
        assert_eq!(last.cleanup, CleanupState::Failed);
        assert!(last.cleaned.is_none());
        assert_eq!(last.cleanup_cost_usd, Some(0.0001));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_new_dictation_replaces_the_old_one_completely() {
        let path = temp_path("replace");
        let store = LastDictationStore::load(path.clone());
        store.begin(transcribed("first", Some("some/model")));
        store.finish_cleanup(Some("First."), 50, 0.001);
        store.finish_paste(900);
        store.begin(transcribed("second", None));
        let last = store.snapshot().unwrap();
        assert_eq!(last.transcript, "second");
        assert_eq!(last.cleanup, CleanupState::Off);
        assert!(last.cleaned.is_none());
        assert!(last.cleanup_ms.is_none() && last.cleanup_cost_usd.is_none());
        assert!(last.cleanup_model.is_none() && last.total_ms.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clear_forgets_it_and_deletes_the_file() {
        let path = temp_path("clear");
        let store = LastDictationStore::load(path.clone());
        store.begin(transcribed("secret", None));
        assert!(path.exists());
        store.clear().unwrap();
        assert!(store.snapshot().is_none());
        assert!(!path.exists());
        store.clear().unwrap(); // clearing nothing is fine
        // A late step of a cleared dictation must not bring it back.
        store.finish_cleanup(Some("x"), 1, 0.0);
        store.finish_paste(1);
        assert!(store.snapshot().is_none());
        assert!(!path.exists());
    }

    #[test]
    fn the_old_text_file_is_removed_on_load() {
        let path = temp_path("legacy");
        let legacy = path.with_extension("txt");
        std::fs::write(&legacy, b"old dictated text").unwrap();
        let _ = LastDictationStore::load(path);
        assert!(!legacy.exists());
    }
}

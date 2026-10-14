//! Usage tally, split by pipeline stage (transcription and clean-up), for the
//! last 24 hours and for all time. Individual events are kept only for the
//! 24 hour window; older ones are folded into the all-time totals.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const WINDOW_SECS: u64 = 24 * 60 * 60;
/// Bumped when the on-disk shape changes. A file with any other version
/// is discarded and the tally starts again from zero.
const FORMAT_VERSION: u32 = 2;

/// Sums for one or more dictations, as stored on disk.
#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq)]
pub struct Totals {
    /// Seconds of audio sent for transcription.
    pub audio_seconds: f64,
    /// Words in the raw transcript.
    pub transcription_words: u64,
    pub transcription_cost_usd: f64,
    /// Words in the cleaned output; zero when clean-up failed.
    pub cleanup_words: u64,
    pub cleanup_cost_usd: f64,
}

impl Totals {
    fn add(&mut self, other: Totals) {
        self.audio_seconds += other.audio_seconds;
        self.transcription_words += other.transcription_words;
        self.transcription_cost_usd += other.transcription_cost_usd;
        self.cleanup_words += other.cleanup_words;
        self.cleanup_cost_usd += other.cleanup_cost_usd;
    }

    fn summary(&self) -> Summary {
        Summary {
            transcription: TranscriptionSummary {
                audio_seconds: self.audio_seconds,
                words: self.transcription_words,
                cost_usd: self.transcription_cost_usd,
            },
            cleanup: CleanupSummary {
                words: self.cleanup_words,
                cost_usd: self.cleanup_cost_usd,
            },
            total_cost_usd: self.transcription_cost_usd + self.cleanup_cost_usd,
        }
    }
}

/// What the settings window shows for one time range.
#[derive(Serialize)]
pub struct Summary {
    pub transcription: TranscriptionSummary,
    pub cleanup: CleanupSummary,
    pub total_cost_usd: f64,
}

#[derive(Serialize)]
pub struct TranscriptionSummary {
    pub audio_seconds: f64,
    pub words: u64,
    pub cost_usd: f64,
}

#[derive(Serialize)]
pub struct CleanupSummary {
    pub words: u64,
    pub cost_usd: f64,
}

#[derive(Serialize)]
pub struct UsageSnapshot {
    pub last_24h: Summary,
    pub all_time: Summary,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct Event {
    /// Unix seconds.
    at: u64,
    totals: Totals,
}

#[derive(Serialize, Deserialize)]
struct UsageFile {
    #[serde(default)]
    version: u32,
    all_time: Totals,
    recent: Vec<Event>,
}

impl Default for UsageFile {
    fn default() -> Self {
        Self { version: FORMAT_VERSION, all_time: Totals::default(), recent: Vec::new() }
    }
}

pub struct UsageStore {
    path: PathBuf,
    data: Mutex<UsageFile>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl UsageStore {
    /// A missing file starts a fresh tally. So does a file in an older
    /// format or one that cannot be read: the tally restarts from zero.
    pub fn load(path: PathBuf) -> Self {
        let existing = std::fs::read(&path).ok();
        let parsed = existing
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<UsageFile>(bytes).ok());
        let data = match parsed {
            Some(file) if file.version == FORMAT_VERSION => file,
            _ => {
                if existing.is_some() {
                    log::info!("usage tally was in an older or unreadable format; starting from zero");
                }
                UsageFile::default()
            }
        };
        Self { path, data: Mutex::new(data) }
    }

    pub fn record(&self, totals: Totals) {
        let mut data = self.data.lock().unwrap();
        let now = now_secs();
        data.all_time.add(totals);
        data.recent.push(Event { at: now, totals });
        data.recent.retain(|e| now.saturating_sub(e.at) < WINDOW_SECS);
        if let Err(e) = self.save(&data) {
            log::warn!("could not save usage tally: {e}");
        }
    }

    pub fn snapshot(&self) -> UsageSnapshot {
        let data = self.data.lock().unwrap();
        let now = now_secs();
        let mut last_24h = Totals::default();
        for event in &data.recent {
            if now.saturating_sub(event.at) < WINDOW_SECS {
                last_24h.add(event.totals);
            }
        }
        UsageSnapshot { last_24h: last_24h.summary(), all_time: data.all_time.summary() }
    }

    /// Write to a temp file then rename, so a crash cannot leave a torn file.
    fn save(&self, data: &UsageFile) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dictation-ai-usage-{name}-{}.json", std::process::id()))
    }

    fn store_with(name: &str, recent: Vec<Event>, all_time: Totals) -> UsageStore {
        UsageStore {
            path: temp_path(name),
            data: Mutex::new(UsageFile { version: FORMAT_VERSION, all_time, recent }),
        }
    }

    /// One dictation: seconds, transcript words, its cost, cleaned words, its cost.
    fn dictation(secs: f64, t_words: u64, t_cost: f64, c_words: u64, c_cost: f64) -> Totals {
        Totals {
            audio_seconds: secs,
            transcription_words: t_words,
            transcription_cost_usd: t_cost,
            cleanup_words: c_words,
            cleanup_cost_usd: c_cost,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn summary_splits_stages_and_sums_the_total() {
        let s = dictation(8.0, 20, 0.0008, 18, 0.0002).summary();
        assert!(close(s.transcription.audio_seconds, 8.0));
        assert_eq!(s.transcription.words, 20);
        assert!(close(s.transcription.cost_usd, 0.0008));
        assert_eq!(s.cleanup.words, 18);
        assert!(close(s.cleanup.cost_usd, 0.0002));
        assert!(close(s.total_cost_usd, 0.0010));
    }

    #[test]
    fn last_24h_excludes_old_events_but_all_time_keeps_them() {
        let now = now_secs();
        let old = dictation(100.0, 200, 0.10, 190, 0.02);
        let new = dictation(10.0, 20, 0.01, 18, 0.002);
        let mut all = old;
        all.add(new);
        let store = store_with(
            "window",
            vec![
                Event { at: now - 2 * WINDOW_SECS, totals: old },
                Event { at: now - 60, totals: new },
            ],
            all,
        );
        let snap = store.snapshot();
        assert_eq!(snap.last_24h.transcription.words, 20);
        assert_eq!(snap.last_24h.cleanup.words, 18);
        assert!(close(snap.last_24h.transcription.audio_seconds, 10.0));
        assert_eq!(snap.all_time.transcription.words, 220);
        assert_eq!(snap.all_time.cleanup.words, 208);
    }

    #[test]
    fn record_adds_to_both_ranges_and_prunes_old_events() {
        let now = now_secs();
        let old = dictation(5.0, 5, 0.005, 5, 0.001);
        let store = store_with(
            "record",
            vec![Event { at: now - 2 * WINDOW_SECS, totals: old }],
            old,
        );
        store.record(dictation(2.0, 3, 0.002, 3, 0.0005));
        let snap = store.snapshot();
        assert_eq!(snap.last_24h.transcription.words, 3);
        assert_eq!(snap.all_time.transcription.words, 8);
        assert_eq!(store.data.lock().unwrap().recent.len(), 1, "old event should be pruned");
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn zero_cost_local_runs_still_count_audio_and_words() {
        let store = store_with("local", Vec::new(), Totals::default());
        store.record(dictation(30.0, 80, 0.0, 78, 0.0004));
        let snap = store.snapshot();
        assert!(close(snap.all_time.transcription.audio_seconds, 30.0));
        assert_eq!(snap.all_time.transcription.words, 80);
        assert!(close(snap.all_time.transcription.cost_usd, 0.0));
        assert!(close(snap.all_time.total_cost_usd, 0.0004));
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn file_in_the_old_unsplit_format_resets_to_zero() {
        let path = temp_path("old-format");
        std::fs::write(
            &path,
            r#"{"all_time":{"audio_seconds":100.0,"words":500,"cost_usd":0.4},
                "recent":[{"at":1,"totals":{"audio_seconds":1.0,"words":2,"cost_usd":0.1}}]}"#,
        )
        .unwrap();
        let store = UsageStore::load(path.clone());
        let snap = store.snapshot();
        assert_eq!(snap.all_time.transcription.words, 0);
        assert!(close(snap.all_time.total_cost_usd, 0.0));
        assert_eq!(snap.last_24h.cleanup.words, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn current_format_survives_a_reload() {
        let path = temp_path("reload");
        let _ = std::fs::remove_file(&path);
        let store = UsageStore::load(path.clone());
        store.record(dictation(12.0, 30, 0.0012, 29, 0.0003));
        drop(store);

        let again = UsageStore::load(path.clone()).snapshot();
        assert_eq!(again.all_time.transcription.words, 30);
        assert_eq!(again.all_time.cleanup.words, 29);
        assert!(close(again.last_24h.total_cost_usd, 0.0015));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unreadable_file_starts_fresh_without_panicking() {
        let path = temp_path("garbage");
        std::fs::write(&path, b"not json at all").unwrap();
        let snap = UsageStore::load(path.clone()).snapshot();
        assert_eq!(snap.all_time.transcription.words, 0);
        let _ = std::fs::remove_file(&path);
    }
}

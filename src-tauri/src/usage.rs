//! Usage tally: audio seconds, words and cost, for the last 24 hours and for
//! all time. Individual events are kept only for the 24 hour window; older
//! ones are folded into the all-time totals.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const WINDOW_SECS: u64 = 24 * 60 * 60;

#[derive(Serialize, Deserialize, Default, Clone, Copy)]
pub struct Totals {
    pub audio_seconds: f64,
    pub words: u64,
    pub cost_usd: f64,
}

impl Totals {
    fn add(&mut self, other: Totals) {
        self.audio_seconds += other.audio_seconds;
        self.words += other.words;
        self.cost_usd += other.cost_usd;
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct Event {
    /// Unix seconds.
    at: u64,
    totals: Totals,
}

#[derive(Serialize, Deserialize, Default)]
struct UsageFile {
    all_time: Totals,
    recent: Vec<Event>,
}

#[derive(Serialize)]
pub struct UsageSnapshot {
    pub last_24h: Totals,
    pub all_time: Totals,
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
    /// A missing or unreadable file starts a fresh tally.
    pub fn load(path: PathBuf) -> Self {
        let data = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
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
        UsageSnapshot { last_24h, all_time: data.all_time }
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

    fn store_with(recent: Vec<Event>, all_time: Totals) -> UsageStore {
        let path = std::env::temp_dir().join(format!("dictation-ai-usage-{}.json", std::process::id()));
        UsageStore {
            path,
            data: Mutex::new(UsageFile { all_time, recent }),
        }
    }

    fn totals(seconds: f64, words: u64, cost: f64) -> Totals {
        Totals { audio_seconds: seconds, words, cost_usd: cost }
    }

    #[test]
    fn last_24h_excludes_old_events_but_all_time_keeps_them() {
        let now = now_secs();
        let store = store_with(
            vec![
                Event { at: now - 2 * WINDOW_SECS, totals: totals(100.0, 200, 0.10) },
                Event { at: now - 60, totals: totals(10.0, 20, 0.01) },
            ],
            totals(110.0, 220, 0.11),
        );
        let snap = store.snapshot();
        assert_eq!(snap.last_24h.words, 20);
        assert!((snap.last_24h.audio_seconds - 10.0).abs() < 1e-9);
        assert_eq!(snap.all_time.words, 220);
    }

    #[test]
    fn record_adds_to_both_and_prunes_old_events() {
        let now = now_secs();
        let store = store_with(
            vec![Event { at: now - 2 * WINDOW_SECS, totals: totals(5.0, 5, 0.005) }],
            totals(5.0, 5, 0.005),
        );
        store.record(totals(2.0, 3, 0.002));
        let snap = store.snapshot();
        assert_eq!(snap.last_24h.words, 3);
        assert_eq!(snap.all_time.words, 8);
        assert_eq!(store.data.lock().unwrap().recent.len(), 1, "old event should be pruned");
        let _ = std::fs::remove_file(&store.path);
    }
}

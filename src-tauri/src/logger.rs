//! Rolling file logger. Total on-disk size is bounded: at most `MAX_FILES`
//! files of at most `MAX_FILE_BYTES` each (5 MB with the defaults).
//!
//! Files: dictation-ai.log (current), dictation-ai.1.log (newest archive)
//! up to dictation-ai.{MAX_FILES-1}.log (oldest, deleted on next rotation).
//!
//! Only Info and above are written to disk. Debug records (which may carry
//! dictated text) go to stderr in debug builds only.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

const BASE_NAME: &str = "dictation-ai";
const MAX_FILE_BYTES: u64 = 1_000_000;
const MAX_FILES: usize = 5;
/// Keeps a single oversized message from defeating the size bound.
const MAX_MESSAGE_CHARS: usize = 2_000;

struct Sink {
    dir: PathBuf,
    file: Option<File>,
    size: u64,
}

impl Sink {
    fn path(&self, index: usize) -> PathBuf {
        if index == 0 {
            self.dir.join(format!("{BASE_NAME}.log"))
        } else {
            self.dir.join(format!("{BASE_NAME}.{index}.log"))
        }
    }

    fn open(&mut self) {
        let _ = fs::create_dir_all(&self.dir);
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(0))
            .ok();
        self.size = self
            .file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map_or(0, |m| m.len());
    }

    fn rotate(&mut self) {
        self.file = None;
        let _ = fs::remove_file(self.path(MAX_FILES - 1));
        for i in (1..MAX_FILES - 1).rev() {
            let _ = fs::rename(self.path(i), self.path(i + 1));
        }
        let _ = fs::rename(self.path(0), self.path(1));
        self.open();
    }

    fn write_line(&mut self, line: &str) {
        let len = line.len() as u64;
        if self.size > 0 && self.size + len > MAX_FILE_BYTES {
            self.rotate();
        }
        if let Some(file) = self.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                self.size += len;
            }
        }
    }
}

struct FileLogger {
    sink: Mutex<Sink>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut message = record.args().to_string();
        if message.chars().count() > MAX_MESSAGE_CHARS {
            message = message.chars().take(MAX_MESSAGE_CHARS).collect::<String>() + "…";
        }
        let line = format!(
            "{} {:<5} {}\n",
            timestamp(),
            record.level(),
            message.replace('\n', " ")
        );

        if cfg!(debug_assertions) {
            eprint!("{line}");
        }
        if record.level() <= Level::Info {
            if let Ok(mut sink) = self.sink.lock() {
                sink.write_line(&line);
            }
        }
    }

    fn flush(&self) {}
}

pub fn init(dir: PathBuf) {
    let mut sink = Sink { dir, file: None, size: 0 };
    sink.open();
    let logger = FileLogger { sink: Mutex::new(sink) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Debug);
    }
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
    }));
}

/// UTC timestamp, e.g. 2026-10-11T14:03:22.123Z.
fn timestamp() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (y, m, day) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        d.subsec_millis()
    )
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dictation-ai-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn dir_size_and_count(dir: &PathBuf) -> (u64, usize) {
        let mut total = 0;
        let mut count = 0;
        for entry in fs::read_dir(dir).unwrap().flatten() {
            total += entry.metadata().unwrap().len();
            count += 1;
        }
        (total, count)
    }

    #[test]
    fn total_size_stays_bounded_and_newest_lines_survive() {
        let dir = temp_dir("rotation");
        let mut sink = Sink { dir: dir.clone(), file: None, size: 0 };
        sink.open();

        // ~20 MB of 100-byte lines: four times the cap.
        let filler = "x".repeat(88);
        for i in 0..200_000 {
            sink.write_line(&format!("{i:08} {filler}\n"));
        }

        let (total, count) = dir_size_and_count(&dir);
        assert!(total <= MAX_FILE_BYTES * MAX_FILES as u64, "total {total} over cap");
        assert!(count <= MAX_FILES, "{count} files");

        let current = fs::read_to_string(dir.join("dictation-ai.log")).unwrap();
        assert!(current.contains("00199999"), "newest line missing");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopening_continues_the_existing_file() {
        let dir = temp_dir("reopen");
        let mut sink = Sink { dir: dir.clone(), file: None, size: 0 };
        sink.open();
        sink.write_line("first\n");
        drop(sink);

        let mut again = Sink { dir: dir.clone(), file: None, size: 0 };
        again.open();
        assert_eq!(again.size, 6);
        again.write_line("second\n");
        let text = fs::read_to_string(dir.join("dictation-ai.log")).unwrap();
        assert_eq!(text, "first\nsecond\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn civil_date_conversion() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_372), (2025, 10, 11));
    }
}

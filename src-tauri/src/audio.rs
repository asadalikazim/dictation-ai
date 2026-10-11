//! Microphone capture. Records mono audio from the default input device while
//! a recording is active, then resamples to 16 kHz and encodes it as WAV.

use std::io::Cursor;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub const TARGET_RATE: u32 = 16_000;
/// Hard cap so a stuck key cannot run up memory or API cost.
const MAX_SECONDS: usize = 180;

/// 16 kHz mono f32 samples in [-1, 1].
pub struct Recording {
    pub samples: Vec<f32>,
}

impl Recording {
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / TARGET_RATE as f32
    }

    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f32 = self.samples.iter().map(|s| s * s).sum();
        (sum_sq / self.samples.len() as f32).sqrt()
    }

    /// Encode as 16-bit PCM WAV.
    pub fn to_wav(&self) -> Result<Vec<u8>, String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TARGET_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).map_err(|e| e.to_string())?;
            for s in &self.samples {
                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                writer.write_sample(v).map_err(|e| e.to_string())?;
            }
            writer.finalize().map_err(|e| e.to_string())?;
        }
        Ok(cursor.into_inner())
    }
}

/// A recording in progress. The cpal stream lives on its own thread because
/// it is not `Send` on macOS.
pub struct ActiveRecording {
    stop_tx: mpsc::Sender<()>,
    result_rx: mpsc::Receiver<Recording>,
}

impl ActiveRecording {
    pub fn stop(self) -> Result<Recording, String> {
        self.stop_tx.send(()).map_err(|_| "recording thread ended early".to_string())?;
        self.result_rx
            .recv()
            .map_err(|_| "recording thread ended early".to_string())
    }
}

/// Names of the available input devices.
pub fn list_input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// Start recording from the named device, or the system default when `None`
/// (or when the named device is no longer connected).
pub fn start(device_name: Option<String>) -> Result<ActiveRecording, String> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (result_tx, result_rx) = mpsc::channel::<Recording>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    thread::spawn(move || match open_stream(device_name.as_deref()) {
        Ok((stream, buffer, source_rate)) => {
            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv();
            drop(stream);
            let raw = std::mem::take(&mut *buffer.lock().unwrap());
            let _ = result_tx.send(Recording {
                samples: resample(&raw, source_rate, TARGET_RATE),
            });
        }
        Err(e) => {
            let _ = ready_tx.send(Err(e));
        }
    });

    ready_rx.recv().map_err(|e| e.to_string())??;
    Ok(ActiveRecording { stop_tx, result_rx })
}

type OpenStream = (cpal::Stream, Arc<Mutex<Vec<f32>>>, u32);

fn open_stream(device_name: Option<&str>) -> Result<OpenStream, String> {
    let host = cpal::default_host();
    let named = device_name.and_then(|wanted| {
        let found = host
            .input_devices()
            .ok()?
            .find(|d| d.name().is_ok_and(|n| n == wanted));
        if found.is_none() {
            log::warn!("input device {wanted:?} not found, using system default");
        }
        found
    });
    let device = named
        .or_else(|| host.default_input_device())
        .ok_or("no input device available")?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
    let max_samples = MAX_SECONDS * sample_rate as usize;

    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, &buffer, channels, max_samples),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, &buffer, channels, max_samples),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, &buffer, channels, max_samples),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, &buffer, channels, max_samples),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, buffer, sample_rate))
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: &Arc<Mutex<Vec<f32>>>,
    channels: usize,
    max_samples: usize,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let buffer = Arc::clone(buffer);
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            let mut buf = buffer.lock().unwrap();
            for frame in data.chunks(channels) {
                if buf.len() >= max_samples {
                    return;
                }
                let sum: f32 = frame.iter().map(|s| s.to_sample::<f32>()).sum();
                buf.push(sum / channels as f32);
            }
        },
        |err| log::error!("audio stream error: {err}"),
        None,
    )
}

/// Box-filter resampler: each output sample averages the input span it
/// covers, which gives basic anti-aliasing when downsampling speech.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let start = (i as f64 * ratio) as usize;
        let end = (((i + 1) as f64 * ratio) as usize).clamp(start + 1, input.len());
        let span = &input[start..end];
        out.push(span.iter().sum::<f32>() / span.len() as f32);
    }
    out
}

use anyhow::{Context, Result};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Lightweight metadata + preview for an audio file.
#[derive(Debug, Clone)]
pub struct DecodedInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub total_frames: u64,
    /// Mono RMS preview samples (one value per ~20ms window).
    pub rms_preview: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct MemoryAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Interleaved f32 PCM: [L, R, L, R, ...]
    pub data: Vec<f32>,
}

/// Events emitted while decoding in the background.
#[derive(Debug)]
pub enum LoadEvent {
    /// Basic metadata is available; a streaming player can start pulling data.
    StreamReady { sample_rate: u32, channels: u16 },
    /// Full preview + PCM finished.
    PreviewReady {
        info: DecodedInfo,
        audio: MemoryAudio,
    },
    /// Fatal error during decoding.
    Error(String),
}

/// Spawn a background thread that streams PCM chunks while computing the preview.
pub fn spawn_decode_job(
    path: PathBuf,
) -> (mpsc::Receiver<LoadEvent>, mpsc::Receiver<Arc<Vec<f32>>>) {
    let (event_tx, event_rx) = mpsc::channel();
    let (pcm_tx, pcm_rx) = mpsc::channel();

    thread::spawn(move || {
        if let Err(err) = decode_streaming(&path, &event_tx, &pcm_tx) {
            let _ = event_tx.send(LoadEvent::Error(format!("{err:#}")));
        }
    });

    (event_rx, pcm_rx)
}

fn decode_streaming(
    path: &Path,
    event_tx: &mpsc::Sender<LoadEvent>,
    pcm_tx: &mpsc::Sender<Arc<Vec<f32>>>,
) -> Result<()> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format
        .default_track()
        .context("no default audio track found")?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let sr = params.sample_rate.context("unknown sample rate")?;
    let chs = params.channels.context("unknown channel count")?.count() as u16;

    event_tx.send(LoadEvent::StreamReady {
        sample_rate: sr,
        channels: chs,
    })?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .context("unsupported codec or failed to build decoder")?;

    let window_frames = (sr / 50).max(1) as usize; // ≈20ms
    let mut rms_preview = Vec::new();
    let mut total_frames: u64 = 0;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut chunk_store: Vec<Arc<Vec<f32>>> = Vec::new();

    // Carry RMS accumulation across packets so there's no dropped tail.
    let mut acc_sq = 0.0f64;
    let mut acc_count = 0usize;

    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                if sample_buf.is_none() {
                    let spec = *audio_buf.spec();
                    let capacity = audio_buf.capacity() as u64;
                    sample_buf = Some(SampleBuffer::<f32>::new(capacity, spec));
                }

                let sbuf = sample_buf.as_mut().unwrap();
                sbuf.copy_interleaved_ref(audio_buf);
                let samples = sbuf.samples(); // interleaved f32
                let chunk = Arc::new(samples.to_vec());
                total_frames += (samples.len() / chs as usize) as u64;

                // push to playback queue
                let _ = pcm_tx.send(chunk.clone());
                chunk_store.push(chunk);

                let chan_count = chs as usize;
                let frames = samples.len() / chan_count;

                for f in 0..frames {
                    let base = f * chan_count;
                    let mut sum = 0.0f32;
                    for c in 0..chan_count {
                        sum += samples[base + c];
                    }
                    let mono = sum / (chan_count as f32);

                    acc_sq += (mono as f64) * (mono as f64);
                    acc_count += 1;

                    if acc_count == window_frames {
                        let rms = (acc_sq / acc_count as f64).sqrt() as f32;
                        rms_preview.push(rms);
                        acc_sq = 0.0;
                        acc_count = 0;
                    }
                }
            }
            Err(SymphoniaError::DecodeError(_)) => continue, // skip corrupt packet
            Err(_) => break,                                 // stop on other errors (incl. EOF)
        }
    }

    if acc_count > 0 {
        let rms = (acc_sq / acc_count as f64).sqrt() as f32;
        rms_preview.push(rms);
    }

    // Build contiguous PCM from chunks for future random access features.
    let total_samples: usize = chunk_store.iter().map(|c| c.len()).sum();
    let mut out = Vec::with_capacity(total_samples);
    for chunk in chunk_store {
        out.extend_from_slice(&chunk);
    }

    let info = DecodedInfo {
        sample_rate: sr,
        channels: chs,
        total_frames,
        rms_preview,
    };

    let audio = MemoryAudio {
        sample_rate: sr,
        channels: chs,
        frames: total_frames,
        data: out,
    };

    event_tx.send(LoadEvent::PreviewReady { info, audio })?;

    Ok(())
}

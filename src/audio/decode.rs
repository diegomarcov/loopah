use anyhow::{Context, Result};
use std::fs::File;
use std::path::Path;

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

/// Probe the file, decode packets once, and compute a downsampled RMS preview.
pub fn probe_and_preview(path: &Path) -> Result<DecodedInfo> {
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

    // --- copy what we need, drop the borrow ---
    let track = format
        .default_track()
        .context("no default audio track found")?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let sr = params.sample_rate.context("unknown sample rate")?;
    let chs = params.channels.context("unknown channel count")?.count() as u16;

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .context("unsupported codec or failed to build decoder")?;

    let window_frames = (sr / 50).max(1) as usize; // ≈20ms
    let mut rms_preview = Vec::new();
    let mut total_frames: u64 = 0;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;

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

                let chan_count = chs as usize;
                let frames = samples.len() / chan_count;
                total_frames += frames as u64;

                // Per-packet RMS accumulation (flushes each full window).
                let mut acc_sq = 0.0f64;
                let mut acc_count = 0usize;

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
                // (We intentionally don't carry partial windows across packets in this MVP.)
            }
            Err(SymphoniaError::DecodeError(_)) => continue, // skip corrupt packet
            Err(_) => break,                                 // stop on other errors (incl. EOF)
        }
    }

    Ok(DecodedInfo {
        sample_rate: sr,
        channels: chs,
        total_frames,
        rms_preview,
    })
}

#[derive(Debug, Clone)]
pub struct MemoryAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Interleaved f32 PCM: [L, R, L, R, ...]
    pub data: Vec<f32>,
}

/// Decode entire file into interleaved f32 in memory.
pub fn decode_all_interleaved(path: &Path) -> Result<MemoryAudio> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

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

    let track = format.default_track().context("no default audio track")?;
    let params = track.codec_params.clone();

    let sr = params.sample_rate.context("unknown sample rate")?;
    let chs = params.channels.context("unknown channel count")?.count() as u16;

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .context("unsupported codec")?;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut out: Vec<f32> = Vec::new();
    let track_id = track.id;

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
                out.extend_from_slice(sbuf.samples());
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    let frames = (out.len() / chs as usize) as u64;
    Ok(MemoryAudio {
        sample_rate: sr,
        channels: chs,
        frames,
        data: out,
    })
}

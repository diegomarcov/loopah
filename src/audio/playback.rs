use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::decode::MemoryAudio;

/// Simple player that streams a decoded buffer to the default output device,
/// resampling linearly if device_sr != src.sample_rate.
pub struct Player {
    _stream: cpal::Stream,
    shared: Arc<Mutex<State>>,
}

struct State {
    src: Arc<MemoryAudio>,
    pos_frame: f64, // fractional frame position in source
    ratio: f64,     // src_sr / dev_sr
    volume: f32,
    playing: bool,
}

impl Player {
    pub fn new(src: MemoryAudio) -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().context("no output device")?;
        let mut config = device.default_output_config()?.config();

        // We output f32, interleaved.
        config.channels = src.channels;
        // Use device's sample rate, compute resample ratio:
        let dev_sr = config.sample_rate.0 as f64;
        let ratio = src.sample_rate as f64 / dev_sr;

        let shared = Arc::new(Mutex::new(State {
            src: Arc::new(src),
            pos_frame: 0.0,
            ratio,
            volume: 1.0,
            playing: true,
        }));

        let shared_cb = Arc::clone(&shared);

        let err_fn = |e| eprintln!("CPAL stream error: {e}");

        let stream = device.build_output_stream(
            &config,
            move |output: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                // 1) Lock briefly and copy what we need into locals.
                //    Then drop the lock before heavy processing to avoid contention.
                let (src, mut pos_frame, ratio, volume, playing) = {
                    let st = match shared_cb.lock() {
                        Ok(g) => g,
                        Err(_) => {
                            // poisoned; fill silence
                            for s in output.iter_mut() {
                                *s = 0.0;
                            }
                            return;
                        }
                    };
                    (
                        Arc::clone(&st.src), // clone the Arc, not a &borrow
                        st.pos_frame,
                        st.ratio,
                        st.volume,
                        st.playing,
                    )
                }; // <-- lock released here

                let ch = src.channels as usize;
                let total_frames = src.frames as usize;

                if !playing || total_frames == 0 || ch == 0 {
                    for s in output.iter_mut() {
                        *s = 0.0;
                    }
                    return;
                }

                // 2) Fill the output buffer with linearly resampled samples.
                let out_frames = output.len() / ch;
                let mut wrote = 0usize;

                for f in 0..out_frames {
                    let p = pos_frame;
                    let i0 = p.floor() as usize;
                    if i0 >= total_frames.saturating_sub(1) {
                        break; // reached (or passed) end
                    }
                    let frac = (p - i0 as f64) as f32;
                    let i1 = i0 + 1;

                    for c in 0..ch {
                        let s0 = src.data[i0 * ch + c];
                        let s1 = src.data[i1 * ch + c];
                        output[f * ch + c] = (s0 + (s1 - s0) * frac) * volume;
                    }

                    pos_frame += ratio;
                    wrote += 1;
                }

                // Zero any tail we didn't fill (e.g., when hitting end of file).
                for s in &mut output[wrote * ch..] {
                    *s = 0.0;
                }

                // 3) Write back the updated playhead (and stop at end).
                if let Ok(mut st) = shared_cb.lock() {
                    st.pos_frame = pos_frame.min(total_frames as f64);
                    if st.pos_frame >= total_frames as f64 {
                        st.playing = false; // stop at end (MVP behavior)
                    }
                }
            },
            err_fn,
            None,
        )?;

        stream.play()?;

        Ok(Self {
            _stream: stream,
            shared,
        })
    }

    pub fn play(&self) {
        if let Ok(mut st) = self.shared.lock() {
            st.playing = true;
        }
    }

    pub fn pause(&self) {
        if let Ok(mut st) = self.shared.lock() {
            st.playing = false;
        }
    }

    pub fn stop(&self) {
        if let Ok(mut st) = self.shared.lock() {
            st.pos_frame = 0.0;
            st.playing = false;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.shared.lock().map(|s| s.playing).unwrap_or(false)
    }
}

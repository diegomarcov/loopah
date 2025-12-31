use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::decode::MemoryAudio;

/// Player that can either stream progressively decoded chunks or play a full buffer.
pub struct Player {
    _stream: cpal::Stream,
    shared: Arc<Mutex<State>>,
}

enum PlaybackMode {
    Memory(MemoryState),
    Stream(StreamState),
}

struct State {
    mode: PlaybackMode,
    playing: bool,
    volume: f32,
}

struct MemoryState {
    src: Arc<MemoryAudio>,
    pos_frame: f64,
    ratio: f64,
    loop_range: Option<(f64, f64)>,
}

struct StreamState {
    receiver: Receiver<Arc<Vec<f32>>>,
    pending: VecDeque<Arc<Vec<f32>>>,
    chunk_offset: usize,
    prev_frame: Vec<f32>,
    next_frame: Vec<f32>,
    initialized: bool,
    phase: f64,
    ratio: f64,
    pos_frame: f64,
    sample_rate: u32,
    channels: u16,
    finished: bool,
}

impl Player {
    pub fn position_seconds(&self) -> f64 {
        if let Ok(st) = self.shared.lock() {
            match &st.mode {
                PlaybackMode::Memory(mem) => mem.pos_frame / (mem.src.sample_rate as f64),
                PlaybackMode::Stream(stream) => stream.pos_frame / (stream.sample_rate as f64),
            }
        } else {
            0.0
        }
    }

    pub fn from_memory(src: MemoryAudio) -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().context("no output device")?;
        let mut config = device.default_output_config()?.config();
        config.channels = src.channels;
        let dev_sr = config.sample_rate.0 as f64;
        let ratio = src.sample_rate as f64 / dev_sr;

        let state = State {
            mode: PlaybackMode::Memory(MemoryState {
                src: Arc::new(src),
                pos_frame: 0.0,
                ratio,
                loop_range: None,
            }),
            playing: true,
            volume: 1.0,
        };

        Self::build_stream(device, config, state)
    }

    pub fn from_stream(
        sample_rate: u32,
        channels: u16,
        receiver: Receiver<Arc<Vec<f32>>>,
    ) -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().context("no output device")?;
        let mut config = device.default_output_config()?.config();
        config.channels = channels;
        let dev_sr = config.sample_rate.0 as f64;
        let ratio = sample_rate as f64 / dev_sr;

        let state = State {
            mode: PlaybackMode::Stream(StreamState {
                receiver,
                pending: VecDeque::new(),
                chunk_offset: 0,
                prev_frame: vec![0.0; channels as usize],
                next_frame: vec![0.0; channels as usize],
                initialized: false,
                phase: 0.0,
                ratio,
                pos_frame: 0.0,
                sample_rate,
                channels,
                finished: false,
            }),
            playing: true,
            volume: 1.0,
        };

        Self::build_stream(device, config, state)
    }

    fn build_stream(
        device: cpal::Device,
        config: cpal::StreamConfig,
        state: State,
    ) -> Result<Self> {
        let shared = Arc::new(Mutex::new(state));
        let shared_cb = Arc::clone(&shared);

        let err_fn = |e| eprintln!("CPAL stream error: {e}");

        let stream = device.build_output_stream(
            &config,
            move |output: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                if let Ok(mut st) = shared_cb.lock() {
                    let playing = st.playing;
                    let volume = st.volume;
                    match &mut st.mode {
                        PlaybackMode::Memory(mem) => process_memory(mem, playing, volume, output),
                        PlaybackMode::Stream(stream) => {
                            process_stream(stream, playing, volume, output)
                        }
                    }
                } else {
                    output.fill(0.0);
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
            st.playing = false;
            match &mut st.mode {
                PlaybackMode::Memory(mem) => {
                    mem.reset_to_loop_start();
                }
                PlaybackMode::Stream(stream) => {
                    stream.pos_frame = 0.0;
                    stream.phase = 0.0;
                    stream.initialized = false;
                    stream.pending.clear();
                    stream.chunk_offset = 0;
                }
            }
        }
    }

    pub fn is_playing(&self) -> bool {
        self.shared.lock().map(|s| s.playing).unwrap_or(false)
    }

    pub fn set_loop(&self, loop_range_secs: Option<(f64, f64)>) {
        if let Ok(mut st) = self.shared.lock() {
            if let PlaybackMode::Memory(mem) = &mut st.mode {
                mem.set_loop(loop_range_secs);
            }
        }
    }

    pub fn set_position_seconds(&self, seconds: f64) {
        if let Ok(mut st) = self.shared.lock() {
            if let PlaybackMode::Memory(mem) = &mut st.mode {
                mem.set_position_seconds(seconds);
            }
        }
    }
}

fn process_memory(mem: &mut MemoryState, playing: bool, volume: f32, output: &mut [f32]) {
    let src = Arc::clone(&mem.src);
    let ch = src.channels as usize;
    if !playing || ch == 0 {
        output.fill(0.0);
        return;
    }

    let total_frames = src.frames as usize;
    let out_frames = output.len() / ch;
    let mut wrote = 0usize;
    for f in 0..out_frames {
        mem.enforce_loop_bounds();
        let p = mem.pos_frame;
        let i0 = p.floor() as usize;
        if i0 >= total_frames.saturating_sub(1) {
            break;
        }
        let frac = (p - i0 as f64) as f32;
        let i1 = i0 + 1;
        for c in 0..ch {
            let s0 = src.data[i0 * ch + c];
            let s1 = src.data[i1 * ch + c];
            output[f * ch + c] = (s0 + (s1 - s0) * frac) * volume;
        }
        mem.pos_frame += mem.ratio;
        mem.enforce_loop_bounds();
        wrote += 1;
    }
    for s in &mut output[wrote * ch..] {
        *s = 0.0;
    }
    if mem.pos_frame >= total_frames as f64 {
        mem.pos_frame = total_frames as f64;
    }
}

fn process_stream(stream: &mut StreamState, playing: bool, volume: f32, output: &mut [f32]) {
    if !playing {
        output.fill(0.0);
        return;
    }

    loop {
        match stream.receiver.try_recv() {
            Ok(chunk) => stream.pending.push_back(chunk),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                stream.finished = true;
                break;
            }
        }
    }

    let ch = stream.channels as usize;
    let mut frame_idx = 0usize;
    let frames_out = output.len() / ch;

    while frame_idx < frames_out {
        if !stream.initialized {
            if !read_frame(
                &mut stream.pending,
                &mut stream.chunk_offset,
                ch,
                &mut stream.prev_frame,
            ) {
                zero_from(output, frame_idx * ch);
                return;
            }
            if !read_frame(
                &mut stream.pending,
                &mut stream.chunk_offset,
                ch,
                &mut stream.next_frame,
            ) {
                zero_from(output, frame_idx * ch);
                return;
            }
            stream.initialized = true;
        }

        let frac = stream.phase as f32;
        for c in 0..ch {
            let s0 = stream.prev_frame[c];
            let s1 = stream.next_frame[c];
            output[frame_idx * ch + c] = (s0 + (s1 - s0) * frac) * volume;
        }

        stream.phase += stream.ratio;
        stream.pos_frame += stream.ratio;
        while stream.phase >= 1.0 {
            stream.phase -= 1.0;
            stream.prev_frame.copy_from_slice(&stream.next_frame);
            if !read_frame(
                &mut stream.pending,
                &mut stream.chunk_offset,
                ch,
                &mut stream.next_frame,
            ) {
                if stream.finished {
                    stream.initialized = false;
                    zero_from(output, (frame_idx + 1) * ch);
                    return;
                } else {
                    zero_from(output, (frame_idx + 1) * ch);
                    return;
                }
            }
        }

        frame_idx += 1;
    }
}

fn read_frame(
    pending: &mut VecDeque<Arc<Vec<f32>>>,
    chunk_offset: &mut usize,
    channels: usize,
    target: &mut [f32],
) -> bool {
    loop {
        let chunk = match pending.front() {
            Some(c) => c,
            None => return false,
        };
        if *chunk_offset + channels > chunk.len() {
            pending.pop_front();
            *chunk_offset = 0;
            continue;
        }
        target.copy_from_slice(&chunk[*chunk_offset..*chunk_offset + channels]);
        *chunk_offset += channels;
        if *chunk_offset >= chunk.len() {
            pending.pop_front();
            *chunk_offset = 0;
        }
        return true;
    }
}

fn zero_from(buf: &mut [f32], start: usize) {
    for s in &mut buf[start..] {
        *s = 0.0;
    }
}

impl MemoryState {
    fn set_loop(&mut self, range_secs: Option<(f64, f64)>) {
        if let Some((start, end)) = range_secs {
            let sr = self.src.sample_rate as f64;
            let mut s = (start * sr).floor();
            let mut e = (end * sr).ceil();
            if e <= s {
                self.loop_range = None;
                return;
            }
            let max_frame = (self.src.frames as f64 - 1.0).max(0.0);
            s = s.clamp(0.0, max_frame);
            e = e.clamp(s + 1.0, self.src.frames as f64);
            if e - s < 1.0 {
                self.loop_range = None;
                return;
            }
            self.loop_range = Some((s, e));
            self.enforce_loop_bounds();
        } else {
            self.loop_range = None;
        }
    }

    fn set_position_seconds(&mut self, seconds: f64) {
        let sr = self.src.sample_rate as f64;
        let frame = (seconds * sr).clamp(0.0, (self.src.frames as f64 - 1.0).max(0.0));
        self.pos_frame = frame;
        self.enforce_loop_bounds();
    }

    fn enforce_loop_bounds(&mut self) {
        if let Some((start, end)) = self.loop_range {
            let span = (end - start).max(1.0);
            if self.pos_frame < start {
                self.pos_frame = start;
            } else if self.pos_frame >= end {
                let offset = (self.pos_frame - start).rem_euclid(span);
                self.pos_frame = start + offset;
            }
        } else {
            self.pos_frame = self
                .pos_frame
                .clamp(0.0, (self.src.frames as f64 - 1.0).max(0.0));
        }
    }

    fn reset_to_loop_start(&mut self) {
        if let Some((start, _)) = self.loop_range {
            self.pos_frame = start;
        } else {
            self.pos_frame = 0.0;
        }
    }
}

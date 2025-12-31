use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use crate::audio::decode::{DecodedInfo, LoadEvent, MemoryAudio, spawn_decode_job};
use crate::audio::playback::Player;
use crate::ui::waveform::{WaveformResult, draw_waveform};

#[derive(Clone, Copy, Debug)]
struct LoopRange {
    start: f64,
    end: f64,
}

impl LoopRange {
    fn ordered(a: f64, b: f64) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    fn clamp(self, duration: f64) -> Self {
        let mut start = self.start.clamp(0.0, duration);
        let mut end = self.end.clamp(0.0, duration);
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        Self { start, end }
    }

    fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

pub struct LoopahApp {
    selected_file: Option<PathBuf>,
    info: Option<DecodedInfo>,
    mem_audio: Option<MemoryAudio>,
    player: Option<Player>,
    load_events: Option<mpsc::Receiver<LoadEvent>>,
    stream_rx: Option<mpsc::Receiver<Arc<Vec<f32>>>>,
    meta_sample_rate: Option<u32>,
    meta_channels: Option<u16>,
    load_error: Option<String>,
    loop_range: Option<LoopRange>,
    loop_drag_anchor: Option<f64>,

    // Waveform view state (seconds):
    view_x_min: f64,
    view_x_max: f64,
}

impl LoopahApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            selected_file: None,
            info: None,
            mem_audio: None,
            player: None,
            load_events: None,
            stream_rx: None,
            meta_sample_rate: None,
            meta_channels: None,
            load_error: None,
            loop_range: None,
            loop_drag_anchor: None,
            view_x_min: 0.0,
            view_x_max: 10.0, // temporary; reset on file open
        }
    }

    fn reset_state(&mut self) {
        self.info = None;
        self.mem_audio = None;
        self.player = None;
        self.load_events = None;
        self.stream_rx = None;
        self.meta_sample_rate = None;
        self.meta_channels = None;
        self.load_error = None;
        self.loop_range = None;
        self.loop_drag_anchor = None;
        self.view_x_min = 0.0;
        self.view_x_max = 10.0;
    }

    fn poll_loader(&mut self) {
        let mut drop_events = false;
        if let Some(rx) = &self.load_events {
            while let Ok(event) = rx.try_recv() {
                match event {
                    LoadEvent::StreamReady {
                        sample_rate,
                        channels,
                    } => {
                        self.meta_sample_rate = Some(sample_rate);
                        self.meta_channels = Some(channels);
                        if let Some(pcm_rx) = self.stream_rx.take() {
                            match Player::from_stream(sample_rate, channels, pcm_rx) {
                                Ok(p) => self.player = Some(p),
                                Err(e) => {
                                    eprintln!("Audio output init failed: {e:#}");
                                }
                            }
                        }
                    }
                    LoadEvent::PreviewReady { info, audio } => {
                        self.view_x_min = 0.0;
                        self.view_x_max =
                            (info.total_frames as f64 / info.sample_rate as f64).max(1.0);
                        self.mem_audio = Some(audio.clone());
                        self.info = Some(info);
                        let duration = file_duration_seconds(self.info.as_ref().unwrap());
                        self.loop_range = Some(LoopRange::ordered(0.0, duration));
                        self.loop_drag_anchor = None;
                        let should_replace = self
                            .player
                            .as_ref()
                            .map(|p| !p.is_streaming() || !p.is_playing())
                            .unwrap_or(true);
                        if should_replace {
                            match Player::from_memory(audio) {
                                Ok(p) => self.player = Some(p),
                                Err(e) => {
                                    eprintln!("Audio output init failed: {e:#}");
                                }
                            }
                        }
                        drop_events = true;
                    }
                    LoadEvent::Error(msg) => {
                        self.load_error = Some(msg);
                        self.stream_rx = None;
                        drop_events = true;
                    }
                }
            }
        }
        if drop_events {
            self.load_events = None;
        }
    }
}

impl eframe::App for LoopahApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_loader();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open audio…").clicked() {
                    let picked = rfd::FileDialog::new()
                        .add_filter("Audio", &["m4a", "aac", "mp3", "wav", "flac"])
                        .pick_file();

                    if let Some(path) = picked {
                        self.reset_state();
                        self.selected_file = Some(path.clone());
                        let (events, stream_rx) = spawn_decode_job(path);
                        self.load_events = Some(events);
                        self.stream_rx = Some(stream_rx);
                    }
                }

                if let Some(p) = &self.selected_file {
                    ui.label(p.display().to_string());
                } else {
                    ui.label("No file selected");
                }
                if let Some(player) = &self.player {
                    if ui
                        .button(if player.is_playing() { "Pause" } else { "Play" })
                        .clicked()
                    {
                        if player.is_playing() {
                            player.pause();
                        } else {
                            player.play();
                        }
                    }
                    if ui.button("Stop").clicked() {
                        player.stop();
                    }
                } else {
                    ui.add_enabled(false, egui::Button::new("Play"));
                    ui.add_enabled(false, egui::Button::new("Stop"));
                }
            });
        });

        egui::TopBottomPanel::top("loop-controls").show(ctx, |ui| {
            if let (Some(info), Some(loop_range)) = (self.info.as_ref(), self.loop_range) {
                let duration = file_duration_seconds(info);
                let frame = 1.0 / info.sample_rate as f64;
                let ten_frames = frame * 10.0;
                let mut start = loop_range.start;
                let mut end = loop_range.end;
                let mut changed = false;

                ui.horizontal(|ui| {
                    ui.label("Loop");

                    ui.label("A");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut start)
                                .speed(frame.max(0.0001))
                                .range(0.0..=duration)
                                .suffix(" s")
                                .max_decimals(3),
                        )
                        .changed();
                    if ui.small_button("−1f").clicked() {
                        start -= frame;
                        changed = true;
                    }
                    if ui.small_button("+1f").clicked() {
                        start += frame;
                        changed = true;
                    }
                    if ui.small_button("−10f").clicked() {
                        start -= ten_frames;
                        changed = true;
                    }
                    if ui.small_button("+10f").clicked() {
                        start += ten_frames;
                        changed = true;
                    }

                    ui.separator();

                    ui.label("B");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut end)
                                .speed(frame.max(0.0001))
                                .range(0.0..=duration)
                                .suffix(" s")
                                .max_decimals(3),
                        )
                        .changed();
                    if ui.small_button("−1f").clicked() {
                        end -= frame;
                        changed = true;
                    }
                    if ui.small_button("+1f").clicked() {
                        end += frame;
                        changed = true;
                    }
                    if ui.small_button("−10f").clicked() {
                        end -= ten_frames;
                        changed = true;
                    }
                    if ui.small_button("+10f").clicked() {
                        end += ten_frames;
                        changed = true;
                    }

                    ui.separator();
                    ui.label(format!("Len: {}", format_time(loop_range.duration())));
                });

                ui.label(
                    egui::RichText::new(
                        "Tip: hold Shift and drag on the waveform to reset A/B quickly.",
                    )
                    .small(),
                );

                if changed {
                    self.loop_range = Some(LoopRange::ordered(start, end).clamp(duration));
                }
            } else {
                ui.label("Load a file to edit loop points.");
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.info.is_some() {
                let (res, duration_for_interaction) = {
                    let info = self.info.as_ref().unwrap();
                    let duration = file_duration_seconds(info);
                    ui.label(format!(
                        "Rate: {} Hz | Ch: {} | Frames: {} | Preview: {} buckets",
                        info.sample_rate,
                        info.channels,
                        info.total_frames,
                        info.rms_preview.len()
                    ));
                    ui.add_space(6.0);
                    let playhead = self.player.as_ref().map(|p| p.position_seconds());
                    let loop_range = self.loop_range.map(|r| (r.start, r.end));
                    (
                        draw_waveform(
                            ui,
                            info,
                            self.view_x_min,
                            self.view_x_max,
                            playhead,
                            loop_range,
                        ),
                        duration,
                    )
                };
                self.view_x_min = res.x_min;
                self.view_x_max = res.x_max;
                self.handle_waveform_interaction(duration_for_interaction, &res);
            } else if let Some(err) = &self.load_error {
                ui.colored_label(egui::Color32::RED, format!("Failed to load audio: {err}"));
            } else if let (Some(sr), Some(ch)) = (self.meta_sample_rate, self.meta_channels) {
                ui.label(format!("Loading preview… {} Hz | Ch: {}", sr, ch));
            } else {
                ui.label("Open an audio file to see its waveform.");
            }
        });
    }
}

impl LoopahApp {
    fn handle_waveform_interaction(&mut self, duration: f64, result: &WaveformResult) {
        if !result.shift_down {
            if result.drag_released {
                self.loop_drag_anchor = None;
            }
            return;
        }
        if result.drag_started {
            if let Some(sec) = result.pointer_seconds {
                self.loop_drag_anchor = Some(sec);
                self.loop_range = Some(LoopRange::ordered(sec, sec).clamp(duration));
            }
        }
        if let (Some(anchor), Some(current)) = (self.loop_drag_anchor, result.pointer_seconds) {
            if result.drag_active {
                self.loop_range = Some(LoopRange::ordered(anchor, current).clamp(duration));
            }
        }
        if result.drag_released {
            self.loop_drag_anchor = None;
        }
    }
}

fn file_duration_seconds(info: &DecodedInfo) -> f64 {
    info.total_frames as f64 / info.sample_rate as f64
}

fn format_time(secs: f64) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as i64;
    let minutes = total_ms / 60_000;
    let seconds = (total_ms % 60_000) / 1000;
    let millis = total_ms % 1000;
    format!("{minutes}:{seconds:02}.{millis:03}")
}

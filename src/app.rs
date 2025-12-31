use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;

use crate::audio::decode::{DecodedInfo, LoadEvent, MemoryAudio, spawn_decode_job};
use crate::audio::playback::Player;
use crate::ui::waveform::draw_waveform;

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

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(info) = &self.info {
                ui.label(format!(
                    "Rate: {} Hz | Ch: {} | Frames: {} | Preview: {} buckets",
                    info.sample_rate,
                    info.channels,
                    info.total_frames,
                    info.rms_preview.len()
                ));
                ui.add_space(6.0);
                let playhead = self.player.as_ref().map(|p| p.position_seconds());
                let res = draw_waveform(ui, info, self.view_x_min, self.view_x_max, playhead);
                self.view_x_min = res.x_min;
                self.view_x_max = res.x_max;
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

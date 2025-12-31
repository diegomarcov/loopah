use eframe::egui;
use std::path::PathBuf;

use crate::audio::decode::{DecodedInfo, MemoryAudio, decode_with_preview};
use crate::audio::playback::Player;
use crate::ui::waveform::draw_waveform;

pub struct LoopahApp {
    selected_file: Option<PathBuf>,
    info: Option<DecodedInfo>,
    mem_audio: Option<MemoryAudio>,
    player: Option<Player>,

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
            view_x_min: 0.0,
            view_x_max: 10.0, // temporary; reset on file open
        }
    }
}

impl eframe::App for LoopahApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open audio…").clicked() {
                    let picked = rfd::FileDialog::new()
                        .add_filter("Audio", &["m4a", "aac", "mp3", "wav", "flac"])
                        .pick_file();

                    if let Some(path) = picked {
                        self.selected_file = Some(path.clone());

                        match decode_with_preview(&path) {
                            Ok((info, mem)) => {
                                self.view_x_min = 0.0;
                                self.view_x_max =
                                    (info.total_frames as f64 / info.sample_rate as f64).max(1.0);
                                self.info = Some(info);
                                self.mem_audio = Some(mem.clone());
                                match Player::new(mem) {
                                    Ok(p) => self.player = Some(p),
                                    Err(e) => {
                                        self.player = None;
                                        eprintln!("Audio output init failed: {e:#}");
                                    }
                                }
                            }
                            Err(err) => {
                                self.info = None;
                                self.mem_audio = None;
                                self.player = None;
                                eprintln!("Failed to decode: {err:#}");
                            }
                        }
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
            } else {
                ui.label("Open an audio file to see its waveform.");
            }
        });
    }
}

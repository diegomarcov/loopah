use eframe::egui;
use std::path::PathBuf;

use crate::audio::decode::{DecodedInfo, MemoryAudio, decode_all_interleaved, probe_and_preview};
use crate::audio::playback::Player;
use crate::ui::waveform::draw_waveform;

pub struct LoopahApp {
    selected_file: Option<PathBuf>,
    info: Option<DecodedInfo>,
    mem_audio: Option<MemoryAudio>,
    player: Option<Player>,
}

impl LoopahApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            selected_file: None,
            info: None,
            mem_audio: None,
            player: None,
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
                        // Decode once to get info + preview.
                        match probe_and_preview(&path) {
                            Ok(info) => self.info = Some(info),
                            Err(err) => {
                                self.info = None;
                                eprintln!("Failed to decode: {err:#}");
                            }
                        }

                        // Decode full PCM for playback
                        match decode_all_interleaved(&path) {
                            Ok(mem) => {
                                self.mem_audio = Some(mem.clone());
                                match Player::new(mem) {
                                    Ok(p) => self.player = Some(p),
                                    Err(e) => {
                                        self.player = None;
                                        eprintln!("Audio output init failed: {e:#}");
                                    }
                                }
                            }
                            Err(e) => {
                                self.mem_audio = None;
                                eprintln!("Full decode failed: {e:#}");
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
                draw_waveform(ui, info);
            } else {
                ui.label("Open an audio file to see its waveform.");
            }
        });
    }
}

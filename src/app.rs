use eframe::egui;
use std::path::PathBuf;

pub struct LoopahApp {
    selected_file: Option<PathBuf>,
}

impl LoopahApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            selected_file: None,
        }
    }
}

impl eframe::App for LoopahApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open audio…").clicked() {
                    self.selected_file = rfd::FileDialog::new()
                        .add_filter("Audio", &["m4a", "aac", "mp3", "wav", "flac"])
                        .pick_file();
                }

                if let Some(p) = &self.selected_file {
                    ui.label(p.display().to_string());
                } else {
                    ui.label("No file selected");
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |_ui| {});
    }
}

mod app;
mod audio;
mod ui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Loopah",
        native_options,
        Box::new(|cc| Ok(Box::new(app::LoopahApp::new(cc)))),
    )
}

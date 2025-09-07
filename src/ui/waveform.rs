use eframe::egui;
use egui_plot::{Line, Plot, PlotBounds, PlotPoints};

use crate::audio::decode::DecodedInfo;

/// Draw a clamped RMS waveform using egui_plot.
pub fn draw_waveform(ui: &mut egui::Ui, info: &DecodedInfo) {
    let n = info.rms_preview.len();
    if n == 0 {
        ui.label("No preview available");
        return;
    }

    let bucket_dt = 1.0_f64 / 50.0; // ~20 ms
    let duration_s = (n as f64) * bucket_dt;

    let points: PlotPoints = (0..n)
        .map(|i| [i as f64 * bucket_dt, info.rms_preview[i] as f64])
        .collect();

    let line = Line::new("RMS", points);

    Plot::new("waveform_plot")
        .height(180.0)
        .allow_boxed_zoom(true)
        .allow_scroll(true)
        .allow_drag(true)
        .include_x(0.0)
        .include_x(duration_s)
        .include_y(-1.0)
        .include_y(1.0)
        .show(ui, |plot_ui| {
            plot_ui.line(line);

            // Clamp current view to [0, duration_s] in X, [-1, 1] in Y.
            let b = plot_ui.plot_bounds();
            let (xmin, xmax) = (0.0, duration_s.max(0.0));
            let (ymin, ymax) = (-1.0, 1.0);

            let nx_min = b.min()[0].clamp(xmin, xmax);
            let nx_max = b.max()[0].clamp(xmin, xmax);
            let ny_min = b.min()[1].clamp(ymin, ymax);
            let ny_max = b.max()[1].clamp(ymin, ymax);

            let clamped = PlotBounds::from_min_max([nx_min, ny_min], [nx_max, ny_max]);
            plot_ui.set_plot_bounds(clamped);
        });
}

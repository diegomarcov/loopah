use eframe::egui;
use egui_plot::{Line, Plot, PlotBounds, PlotPoints, VLine};

use crate::audio::decode::DecodedInfo;

/// Return value for waveform draw: possibly updated X bounds after user panned.
pub struct WaveformResult {
    pub x_min: f64,
    pub x_max: f64,
}

/// Draw a clamped RMS waveform.
/// - Pan: drag inside the plot (horizontal only).
/// - Zoom: managed by parent via passed x_min/x_max (horizontal only).
/// - Y is fixed to [-1, 1].
/// - Optional playhead (seconds) draws a vertical marker.
pub fn draw_waveform(
    ui: &mut egui::Ui,
    info: &DecodedInfo,
    mut x_min: f64,
    mut x_max: f64,
    playhead_sec: Option<f64>,
) -> WaveformResult {
    let n = info.rms_preview.len();
    if n == 0 {
        ui.label("No preview available");
        return WaveformResult {
            x_min: 0.0,
            x_max: 0.0,
        };
    }

    let bucket_dt = 1.0_f64 / 50.0; // ≈20 ms
    let duration_s = (n as f64) * bucket_dt;

    // Clamp incoming bounds to file duration.
    x_min = x_min.clamp(0.0, duration_s);
    x_max = x_max.clamp(0.0, duration_s);
    if x_max <= x_min {
        x_max = (x_min + 1.0).min(duration_s);
    }

    // Build plot points.
    let points: PlotPoints = (0..n)
        .map(|i| [i as f64 * bucket_dt, info.rms_preview[i] as f64])
        .collect();

    let line = Line::new("RMS", points);

    // Build the plot and read back the (possibly) panned bounds.
    let response = Plot::new("waveform_plot")
        .height(180.0)
        .allow_boxed_zoom(false) // disable box zoom
        .allow_scroll(false) // disable wheel zoom (we manage zoom externally)
        .allow_drag(true) // allow panning
        .include_y(-1.0)
        .include_y(1.0)
        .include_x(0.0)
        .include_x(duration_s)
        .show(ui, |plot_ui| {
            // Set starting bounds from parent state.
            let start_bounds = PlotBounds::from_min_max([x_min, -1.0], [x_max, 1.0]);
            plot_ui.set_plot_bounds(start_bounds);

            // Draw waveform.
            plot_ui.line(line);

            // Optional playhead.
            if let Some(t) = playhead_sec {
                let clamped = t.clamp(0.0, duration_s);
                plot_ui.vline(VLine::new("playhead", clamped));
            }

            // Return current (after-user-pan) bounds.
            plot_ui.plot_bounds()
        });

    // Clamp to duration and fixed Y.
    let b = response.inner;
    let nx_min = b.min()[0].clamp(0.0, duration_s);
    let nx_max = b.max()[0].clamp(0.0, duration_s);

    WaveformResult {
        x_min: nx_min,
        x_max: nx_max,
    }
}

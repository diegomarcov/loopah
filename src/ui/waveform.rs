use eframe::egui;
use eframe::egui::{Color32, PointerButton, Stroke};
use egui_plot::{Line, Plot, PlotBounds, PlotPoints, Polygon, VLine};

use crate::audio::decode::DecodedInfo;

/// Return value for waveform draw: possibly updated X bounds after user panned.
pub struct WaveformResult {
    pub x_min: f64,
    pub x_max: f64,
    pub pointer_seconds: Option<f64>,
    pub drag_started: bool,
    pub drag_active: bool,
    pub drag_released: bool,
    pub shift_down: bool,
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
    loop_range: Option<(f64, f64)>,
) -> WaveformResult {
    let n = info.rms_preview.len();
    if n == 0 {
        ui.label("No preview available");
        return WaveformResult {
            x_min: 0.0,
            x_max: 0.0,
            pointer_seconds: None,
            drag_started: false,
            drag_active: false,
            drag_released: false,
            shift_down: false,
        };
    }

    let shift_down = ui.input(|i| i.modifiers.shift);
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
        .allow_drag([!shift_down, false]) // disable drag while selecting loop
        .include_y(-1.0)
        .include_y(1.0)
        .include_x(0.0)
        .include_x(duration_s)
        .show(ui, |plot_ui| {
            // Set starting bounds from parent state.
            let start_bounds = PlotBounds::from_min_max([x_min, -1.0], [x_max, 1.0]);
            plot_ui.set_plot_bounds(start_bounds);

            if let Some((a, b)) = loop_range {
                let start = a.min(b).clamp(0.0, duration_s);
                let end = b.max(a).clamp(0.0, duration_s);
                if end > start {
                    let fill_points: PlotPoints =
                        vec![[start, -1.0], [start, 1.0], [end, 1.0], [end, -1.0]].into();
                    let color = Color32::from_rgba_unmultiplied(120, 180, 255, 48);
                    let polygon = Polygon::new("loop_range_fill", fill_points)
                        .fill_color(color)
                        .stroke(Stroke::NONE);
                    plot_ui.polygon(polygon);
                }
                let marker_color = Color32::from_rgb(120, 180, 255);
                plot_ui.vline(VLine::new("loop_start", start).color(marker_color));
                plot_ui.vline(VLine::new("loop_end", end).color(marker_color));
            }

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
    let pointer_seconds = response
        .response
        .interact_pointer_pos()
        .or_else(|| response.response.hover_pos())
        .map(|pos| response.transform.value_from_position(pos).x);

    WaveformResult {
        x_min: nx_min,
        x_max: nx_max,
        pointer_seconds,
        drag_started: response.response.drag_started_by(PointerButton::Primary),
        drag_active: response.response.dragged_by(PointerButton::Primary),
        drag_released: response.response.drag_stopped_by(PointerButton::Primary),
        shift_down,
    }
}

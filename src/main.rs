//! Qwertone — play music on your laptop keyboard.

mod audio;
mod config;
mod devices;
mod keymap;
mod latency;
mod music;
mod theme;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Qwertone")
            // Hyprland window rules can match on `class:^(qwertone)$`.
            .with_app_id("qwertone")
            .with_inner_size([1180.0, 820.0])
            .with_min_inner_size([760.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Qwertone",
        options,
        Box::new(|cc| Ok(Box::new(ui::app::QwertoneApp::new(cc)))),
    )
}

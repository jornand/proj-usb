#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod app;
mod dialogs;
mod drives;
mod explorer;
mod fs_worker;
mod icons;
#[cfg(test)]
mod integration_tests;
mod model;
mod process;
mod settings;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Projector USB — подготовка видео")
            .with_inner_size([1920.0, 1024.0])
            .with_min_inner_size([1500.0, 750.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Projector USB",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

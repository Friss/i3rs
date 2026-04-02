//! MoTeC telemetry data analysis application.

mod app;
mod panels;
mod state;
mod workspace;

fn main() -> eframe::Result {
    let file_path = std::env::args().nth(1).map(std::path::PathBuf::from);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("MoTeC Data Analysis"),
        ..Default::default()
    };

    eframe::run_native(
        "i3rs-app",
        native_options,
        Box::new(move |cc| {
            let mut app = app::App::new(cc);
            if let Some(path) = file_path {
                app.open_file(path);
            }
            Ok(Box::new(app))
        }),
    )
}

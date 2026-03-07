mod app;
mod bridge;
mod table_view;

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native("dui", options, Box::new(|cc| Ok(Box::new(app::DuiApp::new(cc)))))
}

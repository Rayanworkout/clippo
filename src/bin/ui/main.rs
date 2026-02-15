mod clippo_app;
mod config;
mod ui;

use std::sync::Arc;

use clippo_app::ClippoApp;
use eframe::egui;

const DAEMON_LISTENING_PORT: u32 = 7878;
const DAEMON_SENDING_PORT: u32 = 7879;

fn main() -> eframe::Result<()> {
    // Init logging
    tracing_subscriber::fmt::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([350., 450.])
            .with_max_inner_size([350., 450.])
            .with_maximize_button(false)
            .with_min_inner_size([200., 300.])
            .with_position([250., 340.]),
        centered: true,
        ..Default::default()
    };

    // Create a ClippoApp instance normally (not wrapped in an Arc).
    let clippo_ui = Arc::new(ClippoApp::new());

    // Spawn a background thread that periodically updates the shared history.
    Arc::clone(&clippo_ui).listen_for_history_updates();

    tracing::info!("Starting App ...");

    // Pass the ClippoApp instance directly to run_native.
    eframe::run_native(
        "Clippo",
        options,
        // We clone the inner value of Arc<ClippoApp> because Arc<ClippoApp> does not implement eframe::App
        Box::new(move |_cc| Ok(Box::new((*clippo_ui).clone()))),
    )
}

mod clipboard_daemon;

use std::sync::Arc;

use anyhow::Result;
use clipboard_daemon::Clippo;

const UI_SENDING_PORT: u32 = 7878;
const UI_LISTENING_PORT: u32 = 7879;

fn main() -> Result<()> {
    // Init logging
    tracing_subscriber::fmt::init();

    let clippo = Arc::new(Clippo::new()?);

    // Spawn the UI listener thread. This works because listen_for_ui expects an Arc<Self>.
    tracing::info!("Clippo listening for UI requests on 127.0.0.1:{UI_LISTENING_PORT} ...");
    Arc::clone(&clippo).listen_for_ui();

    // Main thread
    tracing::info!("Clippo listening for clipboard changes and ready to send to UI on 127.0.0.1:{UI_SENDING_PORT} ...");
    clippo.monitor_clipboard_events()?;

    Ok(())
}

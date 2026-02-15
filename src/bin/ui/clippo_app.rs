use crate::config::ClippoConfig;
use crate::DAEMON_LISTENING_PORT;
use crate::DAEMON_SENDING_PORT;
use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use eframe::egui;
use ron::de::from_str;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone)]
pub struct ClippoApp {
    pub history_cache: Arc<Mutex<Vec<String>>>,
    pub search_query: String,
    pub config: ClippoConfig,
    pub style_needs_update: bool,
}

impl ClippoApp {
    pub fn new() -> Self {
        let empty_cache = Vec::new();

        let clippo = ClippoApp {
            history_cache: Arc::new(Mutex::new(empty_cache)),
            search_query: String::new(),
            config: confy::load("clippo", None).unwrap_or_default(),
            style_needs_update: true,
        };

        if let Err(initial_history_error) = clippo.fill_initial_history() {
            tracing::error!("An error occured when loading initial history in Clippo UI: {initial_history_error}.");
        }

        clippo
    }

    /// This method is used inside the UI (preferences)
    /// to toggle / edit config values.
    pub fn toggle_config_field(&mut self, field_name: &str) {
        let allowed_settings: Vec<&str> = vec![
            "minimize_on_copy",
            "minimize_on_clear",
            "dark_mode",
            "max_entry_display_length",
            "enable_search",
        ];

        if !allowed_settings.contains(&field_name) {
            tracing::error!("An invalid value was passed to ClippoApp.toggle_config_field()");
            return;
        }

        // Save the updated configuration
        let _ = confy::store("clippo", None, &self.config);

        // Log the change
        tracing::info!("{field_name} changed in config.");
    }

    /// Helper method to display a single history entry.
    /// It is called within the loop iterating through clipboard history
    pub fn display_history_entry(&self, ui: &mut egui::Ui, ctx: &egui::Context, value: &str) {
        ui.vertical_centered_justified(|ui| {
            // We create a short version of the value but
            // we keep the original to be copied
            let short_value = if value.len() > self.config.max_entry_display_length {
                let truncated: String = value
                    .chars()
                    .take(self.config.max_entry_display_length)
                    .collect();
                format!("{}...", truncated)
            } else {
                value.to_string()
            };

            if ui.button(short_value).clicked() {
                if let Ok(mut clipboard) = Clipboard::new() {
                    match clipboard.set_text(value) {
                        Ok(()) => {
                            tracing::info!("Successfully set value to clipboard.");
                        }
                        Err(e) => {
                            tracing::error!("Could not set clipboard value on click: {e}");
                        }
                    }
                }

                if self.config.minimize_on_copy {
                    // Minimize after copying
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
            ui.add_space(10.0);
        });
    }

    pub fn listen_for_history_updates(self: Arc<Self>) {
        let clippo_app = Arc::clone(&self);
        thread::spawn(move || -> Result<()> {
            let listener = TcpListener::bind(format!("127.0.0.1:{DAEMON_LISTENING_PORT}"))
                .context(format!(
                    "Could not bind to 127.0.0.1:{DAEMON_LISTENING_PORT} when trying to listen for daemon history updates."
                ))?;

            tracing::info!("UI server listening on port {DAEMON_LISTENING_PORT} ...");

            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let mut buffer = Vec::new();

                        stream
                            .read_to_end(&mut buffer)
                            .context("Failed to read from stream")?;
                        let request = String::from_utf8_lossy(&buffer);

                        let mut history = clippo_app
                            .history_cache
                            .lock()
                            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

                        *history =
                            from_str(&request).context("Failed to parse history with RON")?;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to accept connexion on {DAEMON_LISTENING_PORT}: {e} ..."
                        );
                    }
                }
            }
            Ok(())
        });
    }

    /// Fetch the initial history from the daemon with a
    /// TCP request. Uses an empty history if it fails.
    fn fill_initial_history(&self) -> Result<()> {
        let request_result = (|| -> Result<String> {
            let mut stream = TcpStream::connect(format!("127.0.0.1:{DAEMON_SENDING_PORT}"))
                .context(format!(
                "Initial history request could not bind to \"127.0.0.1:{DAEMON_SENDING_PORT}\"."
            ))?;

            stream
                .write_all("GET_HISTORY\n".as_bytes())
                .context("Failed to write to stream when trying to get initial history.")?;

            // Read the server's response into a string.
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .context("Failed to read from stream when trying to get initial history.")?;

            Ok(response)
        })();

        let mut history = self
            .history_cache
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

        if let Ok(old_history) = request_result {
            *history =
                from_str(&old_history).context("Failed to parse initial history with RON")?;
        } else {
            *history = from_str("")?;
            tracing::error!("Could not fetch history from clipboard daemon.\nFalling back to an empty history.\n");
        }
        tracing::info!("Successfully loaded initial history from clipboard daemon ...");
        Ok(())
    }

    pub fn clear_history(&mut self) -> Result<()> {
        let mut history = self
            .history_cache
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

        history.clear();

        let request_result = (|| -> Result<String> {
            let mut stream = TcpStream::connect(format!("127.0.0.1:{DAEMON_SENDING_PORT}"))
                .context(format!(
                    "Clear history request could not bind to \"127.0.0.1:{DAEMON_SENDING_PORT}\"."
                ))?;

            // Send the RESET_HISTORY request to the server
            stream
                .write("RESET_HISTORY\n".as_bytes())
                .expect("Failed to write to stream when trying to clear history.");

            // Read the server's response into a string.
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .context("Failed to read from stream when trying to clear history.")?;
            Ok(response)
        })();

        if let Err(e) = request_result {
            tracing::error!("Could not clear history: {e}\n");
        }

        Ok(())
    }
}

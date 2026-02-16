use crate::config::ClippoConfig;
use crate::DAEMON_LISTENING_PORT;
use crate::DAEMON_SENDING_PORT;
use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use ron::de::from_str;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

#[derive(Clone)]
pub struct ClippoApp {
    pub history_cache: Arc<Mutex<Vec<String>>>,
    pub search_query: String,
    pub config: ClippoConfig,
    pub style_needs_update: bool,
    pub last_action: Option<(String, Instant)>,
    pub confirm_clear: bool,
    pub search_focus_requested: bool,
    pub selected_entry_index: Option<usize>,
}

impl ClippoApp {
    pub fn new() -> Self {
        let empty_cache = Vec::new();

        let clippo = ClippoApp {
            history_cache: Arc::new(Mutex::new(empty_cache)),
            search_query: String::new(),
            config: confy::load("clippo", None).unwrap_or_default(),
            style_needs_update: true,
            last_action: None,
            confirm_clear: false,
            search_focus_requested: false,
            selected_entry_index: None,
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

    pub fn copy_to_clipboard(&self, value: &str) -> Result<()> {
        let mut clipboard = Clipboard::new().context("Could not initialize clipboard backend.")?;
        clipboard
            .set_text(value)
            .context("Could not set clipboard value.")?;
        tracing::info!("Successfully set value to clipboard.");
        Ok(())
    }

    pub fn preview_entry(&self, value: &str) -> String {
        let flat = value.replace('\n', " ").replace('\r', "");
        if flat.chars().count() > self.config.max_entry_display_length {
            let truncated: String = flat
                .chars()
                .take(self.config.max_entry_display_length)
                .collect();
            format!("{truncated}...")
        } else {
            flat
        }
    }

    pub fn set_last_action<S: Into<String>>(&mut self, message: S) {
        self.last_action = Some((message.into(), Instant::now()));
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
            history.clear();
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
                .write_all("RESET_HISTORY\n".as_bytes())
                .context("Failed to write to stream when trying to clear history.")?;

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

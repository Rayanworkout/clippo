use crate::UI_LISTENING_PORT;
use crate::UI_SENDING_PORT;

use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use core::panic;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::{thread, time::Duration};

const HISTORY_FILE_PATH: &str = ".clipboard_history.ron";
const MAX_HISTORY_LENGTH: usize = 100;
const CLIPBOARD_REFRESH_RATE_MS: u64 = 800;

const STREAM_MAX_RETRIES: u32 = 5;
pub struct Clippo {
    clipboard: Mutex<Clipboard>,
    history: Mutex<Vec<String>>,
}

impl Clippo {
    pub fn new() -> Result<Self> {
        // Instanciate a clipboard object that will be used to access
        // or update the system clipboard.

        // We load the old history when instanciating
        // a new object to ensure history persistance
        Ok(Self {
            clipboard: Clipboard::new().context("Could not create a clipboard instance, the listener daemon can not run: {clipboard_error}")?.into(),
            history: Self::load_history()?.into(),
        })
    }

    /// Monitor clipboard changes and send a request to the UI on copy.
    pub fn monitor_clipboard_events(&self) -> Result<()> {
        let mut consecutive_clipboard_failures = 0;

        loop {
            if let Ok(mut clipboard) = self.clipboard.lock() {
                match clipboard.get_text() {
                    Ok(content) => {
                        if consecutive_clipboard_failures > 0 {
                            consecutive_clipboard_failures = 0
                        }

                        let mut history = self
                            .history
                            .lock()
                            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

                        if !history.contains(&content) && !content.trim().is_empty() {
                            // Insert new value at first index
                            history.insert(0, content);

                            let history_len = history.len();
                            // Keep only the wanted number of entries
                            if history_len > MAX_HISTORY_LENGTH {
                                history.pop();
                            }

                            // Explicitly drop the lock otherwise save_history() won't be
                            // able to access the variable
                            drop(history);

                            // Send the TCP request to the UI
                            match TcpStream::connect(format!("127.0.0.1:{UI_SENDING_PORT}")) {
                                Ok(stream) => match self.send_history(stream) {
                                    Ok(()) => {
                                        tracing::info!(
                                        "Successfully sent history to UI after clipboard event ..."
                                    );
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "An error occured when sending history to UI after clipboard event: {e} ..."
                                        );
                                    }
                                },
                                Err(_) => {
                                    // UI not available
                                }
                            }

                            // Save new history to file
                            match self.save_history() {
                                Ok(()) => {
                                    tracing::info!(
                                        "Successfully saved history after clipboard event ..."
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "An error occured when saving history to file after clipboard event: {e} ..."
                                    );
                                }
                            }
                        }
                    }
                    Err(clipboard_content_error) => {
                        tracing::error!(
                            "Error getting the clipboard content: {clipboard_content_error}"
                        );
                        consecutive_clipboard_failures += 1;
                        // Setting an empty value to the clipboard in case it is empty, to prevent errors
                        tracing::info!("Setting an arbitrary value to the clipboard to prevent read issues ...");
                        clipboard.set_text("Hello Clippo !")?;

                        if consecutive_clipboard_failures == 3 {
                            panic!("Error getting the clipboard content 3 times in a row, aborting daemon run.")
                        }

                        thread::sleep(Duration::from_millis(CLIPBOARD_REFRESH_RATE_MS));
                    }
                }
            }

            thread::sleep(Duration::from_millis(CLIPBOARD_REFRESH_RATE_MS));
        }
    }

    /// Listen for directives coming from the UI for example clear_history() or the initial
    /// history request when starting. This way the UI can stop and start while always
    /// having an up to date history as long as the clipboard daemon is running.
    /// We use a simple retry mechanism in case some requests fail.
    pub fn listen_for_ui(self: Arc<Self>) {
        let clippo = Arc::clone(&self);
        thread::spawn(move || -> Result<()> {
            let mut buffer = [0; 512];

            let listener = TcpListener::bind(format!("127.0.0.1:{UI_LISTENING_PORT}")).context(
                format!("UI listener could not bind to \"127.0.0.1:{UI_SENDING_PORT}\"."),
            )?;

            let mut get_stream_consecutive_failures = 0;
            for stream in listener.incoming() {
                let stream_success_result = (|| -> Result<()> {
                    let mut stream =
                        stream.context("Could not get stream from incoming UI connexion.")?;
                    let size = stream
                        .read(&mut buffer)
                        .context("Could not read the incoming request from the UI.")?;

                    let request = String::from_utf8_lossy(&buffer[..size]);

                    if request.trim() == "GET_HISTORY" {
                        clippo
                            .send_history(stream.try_clone()?)
                            .context("Could not send the history to UI, stream.write() failed.")?;

                        tracing::info!(
                            "\"GET_HISTORY\" request received, sending current history to UI ..."
                        );
                    } else if request.trim() == "RESET_HISTORY" {
                        clippo
                            .clear_history()
                            .context("Could not clear history after UI request.")?;

                        stream.write(b"OK")?;

                        tracing::info!(
                            "\"RESET_HISTORY\" request received, clearing current history ..."
                        );
                    } else {
                        stream.write(b"BAD_REQUEST")?;
                        tracing::warn!(
                            "Unexpected request received, sending back \"BAD_REQUEST\" to the UI ..."
                        );
                    }
                    Ok(())
                })();

                match stream_success_result {
                    Ok(()) => {
                        // Reset the failure counter on success.
                        get_stream_consecutive_failures = 0;
                    }
                    Err(e) => {
                        tracing::error!("Error handling UI request: {e}. Retrying...");
                        get_stream_consecutive_failures += 1;
                        if get_stream_consecutive_failures >= STREAM_MAX_RETRIES {
                            tracing::error!("Exceeded {STREAM_MAX_RETRIES} consecutive failures. Exiting UI listener thread.");

                            panic!(
                                "Exceeded {STREAM_MAX_RETRIES} consecutive failures. Exiting UI listener thread.",
                            );
                        }
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            }
            Ok(())
        });
    }

    /// Save clipboard history to ron file.
    fn save_history(&self) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(HISTORY_FILE_PATH)
            .context(format!("Could not create or open {HISTORY_FILE_PATH}"))?;

        let history = self
            .history
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

        let history_data = Vec::from(history.clone());
        let serialized_history = ron::ser::to_string(&history_data)
            .context("Could not serialize history when saving to file.")?;

        file.write_all(serialized_history.as_bytes())
            .context(format!(
                "Could not write serialized history to {HISTORY_FILE_PATH}"
            ))?;

        Ok(())
    }

    /// Loads the current history from the file.
    /// Static method.
    fn load_history() -> Result<Vec<String>> {
        let history: Vec<String> = fs::File::open(HISTORY_FILE_PATH)
            // We add some context to the rror in case we cannot open the file
            .context(format!("Could not open \"{HISTORY_FILE_PATH}\""))
            // And we chain an operation to deserialize the content if the opening works
            .and_then(|file| {
                let reader = BufReader::new(file);
                ron::de::from_reader(reader).context("Error deserializing clipboard history.")
            })
            // if any of these steps fail, we fall back to an empty Vec<String> and notify the user
            .unwrap_or_else(|load_error| {
                eprintln!(
                    "Could not load history: {load_error}\nFalling back to an empty history.\n",
                );
                Vec::new()
            });

        Ok(history)
    }

    fn clear_history(&self) -> Result<()> {
        let mut history = self
            .history
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

        history.clear(); // Clear history in memory
        fs::remove_file(HISTORY_FILE_PATH).context("Could not delete the history file.")?;

        // We could also clear the current state of the keyboard
        // self.clipboard.clear()?;
        Ok(())
    }

    fn send_history(&self, mut stream: TcpStream) -> Result<()> {
        let history = self
            .history
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

        for attempt in 0..STREAM_MAX_RETRIES {
            let send_result = (|| -> Result<()> {
                stream.write_all(format!("{:?}\n", history).as_bytes())?;
                stream
                    .shutdown(Shutdown::Write)
                    .context("Could not close the TCP connection when sending history.")?;
                Ok(())
            })();

            match send_result {
                Ok(()) => return Ok(()),
                Err(e) => {
                    eprintln!(
                        "Could not send history to UI on attempt {}/{}: {}. Retrying...",
                        attempt + 1,
                        STREAM_MAX_RETRIES,
                        e
                    );
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }

        Err(anyhow::anyhow!(
            "Could not send history to UI {} times in a row",
            STREAM_MAX_RETRIES
        ))
    }
}

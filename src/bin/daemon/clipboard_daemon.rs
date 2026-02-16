use crate::UI_LISTENING_PORT;
use crate::UI_SENDING_PORT;

use anyhow::{anyhow, Context, Result};
use arboard::{Clipboard, Error as ClipboardError, ImageData};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::{thread, time::Duration};

const HISTORY_FILE_PATH: &str = ".clipboard_history.ron";
const MAX_HISTORY_LENGTH: usize = 100;
const CLIPBOARD_REFRESH_RATE_MS: u64 = 800;

const STREAM_MAX_RETRIES: u32 = 5;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum ClipboardHistoryEntry {
    Text(String),
    Image(ClipboardImageEntry),
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ClipboardImageEntry {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
}

impl ClipboardImageEntry {
    fn from_image_data(image: ImageData<'_>) -> Self {
        Self {
            width: image.width,
            height: image.height,
            bytes: image.bytes.into_owned(),
        }
    }
}

pub struct Clippo {
    clipboard: Mutex<Clipboard>,
    history: Mutex<Vec<ClipboardHistoryEntry>>,
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
        loop {
            if let Ok(mut clipboard) = self.clipboard.lock() {
                match Self::read_clipboard_entry(&mut clipboard) {
                    Ok(Some(entry)) => {
                        let mut history = self
                            .history
                            .lock()
                            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

                        if !history.contains(&entry) {
                            // Insert new value at first index
                            history.insert(0, entry);

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
                    Ok(None) => {}
                    Err(read_clipboard_error) => {
                        tracing::error!(
                            "Error getting clipboard content in supported formats: {read_clipboard_error}"
                        );
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

        let serialized_history = ron::ser::to_string(&*history)
            .context("Could not serialize history when saving to file.")?;

        file.write_all(serialized_history.as_bytes())
            .context(format!(
                "Could not write serialized history to {HISTORY_FILE_PATH}"
            ))?;

        Ok(())
    }

    /// Loads the current history from the file.
    /// Static method.
    fn load_history() -> Result<Vec<ClipboardHistoryEntry>> {
        let typed_history_result: Result<Vec<ClipboardHistoryEntry>> =
            fs::File::open(HISTORY_FILE_PATH)
                .context(format!("Could not open \"{HISTORY_FILE_PATH}\""))
                .and_then(|file| {
                    let reader = BufReader::new(file);
                    ron::de::from_reader(reader).context("Error deserializing clipboard history.")
                });

        match typed_history_result {
            Ok(history) => Ok(history),
            Err(typed_load_error) => {
                let legacy_history_result: Result<Vec<String>> = fs::File::open(HISTORY_FILE_PATH)
                    .context(format!("Could not open \"{HISTORY_FILE_PATH}\""))
                    .and_then(|file| {
                        let reader = BufReader::new(file);
                        ron::de::from_reader(reader)
                            .context("Error deserializing legacy clipboard history.")
                    });

                match legacy_history_result {
                    Ok(legacy_history) => {
                        tracing::warn!(
                            "Loaded legacy string-only clipboard history format in daemon; data will be migrated on next save."
                        );
                        Ok(legacy_history
                            .into_iter()
                            .map(ClipboardHistoryEntry::Text)
                            .collect())
                    }
                    Err(load_error) => {
                        eprintln!(
                            "Could not load typed history: {typed_load_error}\nCould not load legacy history: {load_error}\nFalling back to an empty history.\n",
                        );
                        Ok(Vec::new())
                    }
                }
            }
        }
    }

    fn clear_history(&self) -> Result<()> {
        let mut history = self
            .history
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

        history.clear(); // Clear history in memory
        match fs::remove_file(HISTORY_FILE_PATH) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(anyhow!("Could not delete the history file: {error}"));
            }
        }

        // We could also clear the current state of the keyboard
        // self.clipboard.clear()?;
        Ok(())
    }

    fn send_history(&self, mut stream: TcpStream) -> Result<()> {
        let history = self
            .history
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {}", e))?;

        let serialized_history = ron::ser::to_string(&*history)
            .context("Could not serialize history when sending to UI.")?;

        for attempt in 0..STREAM_MAX_RETRIES {
            let send_result = (|| -> Result<()> {
                stream.write_all(serialized_history.as_bytes())?;
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

    fn read_clipboard_entry(clipboard: &mut Clipboard) -> Result<Option<ClipboardHistoryEntry>> {
        match clipboard.get_text() {
            Ok(content) => {
                if !content.trim().is_empty() {
                    return Ok(Some(ClipboardHistoryEntry::Text(content)));
                }
            }
            Err(ClipboardError::ContentNotAvailable) => {}
            Err(text_error) => {
                return Err(anyhow!(
                    "Could not get clipboard text content: {text_error}"
                ));
            }
        }

        match clipboard.get_image() {
            Ok(image) => Ok(Some(ClipboardHistoryEntry::Image(
                ClipboardImageEntry::from_image_data(image),
            ))),
            Err(ClipboardError::ContentNotAvailable) => Ok(None),
            Err(image_error) => Err(anyhow!(
                "Could not get clipboard image content: {image_error}"
            )),
        }
    }
}

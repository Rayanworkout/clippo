use crate::UI_LISTENING_PORT;
use crate::UI_SENDING_PORT;

use anyhow::{anyhow, Context, Result};
use arboard::{Clipboard, Error as ClipboardError, ImageData};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
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
    capture_state: Mutex<CaptureState>,
}

#[derive(Default)]
struct CaptureState {
    paused: bool,
    skip_next_read: bool,
    suppressed_entry: Option<ClipboardHistoryEntry>,
}

impl CaptureState {
    fn set_paused(&mut self, paused: bool) {
        if self.paused && !paused {
            self.skip_next_read = true;
        }
        self.paused = paused;
    }

    fn should_capture(&mut self, entry: &ClipboardHistoryEntry) -> bool {
        if self.suppressed_entry.as_ref() == Some(entry) {
            return false;
        }
        self.suppressed_entry = None;
        true
    }
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
            capture_state: Mutex::new(CaptureState::default()),
        })
    }

    /// Monitor clipboard changes and send a request to the UI on copy.
    pub fn monitor_clipboard_events(&self) -> Result<()> {
        let mut last_seen = None;
        loop {
            let mut capture_state = self
                .capture_state
                .lock()
                .map_err(|e| anyhow!("Could not acquire capture state lock: {e}"))?;
            if capture_state.paused {
                drop(capture_state);
                thread::sleep(Duration::from_millis(CLIPBOARD_REFRESH_RATE_MS));
                continue;
            }

            let mut history_changed = false;
            if let Ok(mut clipboard) = self.clipboard.lock() {
                match Self::read_clipboard_entry(&mut clipboard) {
                    Ok(Some(entry)) => {
                        // shortcut: polling cannot detect copying unchanged content, use platform events if repeat copies must count.
                        let should_capture = capture_state.should_capture(&entry);
                        if capture_state.skip_next_read {
                            last_seen = Some(entry);
                            capture_state.skip_next_read = false;
                        } else if last_seen.as_ref() != Some(&entry) {
                            last_seen = Some(entry.clone());
                            if should_capture {
                                let mut history = self
                                    .history
                                    .lock()
                                    .map_err(|e| anyhow!("Could not acquire history lock: {e}"))?;
                                if !history.contains(&entry) {
                                    history.insert(0, entry);
                                    if history.len() > MAX_HISTORY_LENGTH {
                                        history.pop();
                                    }
                                    history_changed = true;
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        last_seen = None;
                        capture_state.skip_next_read = false;
                        capture_state.suppressed_entry = None;
                    }
                    Err(read_clipboard_error) => {
                        tracing::error!(
                            "Error getting clipboard content in supported formats: {read_clipboard_error}"
                        );
                    }
                }
            }
            drop(capture_state);

            if history_changed {
                if let Ok(stream) = TcpStream::connect(format!("127.0.0.1:{UI_SENDING_PORT}")) {
                    if let Err(error) = self.send_history(stream) {
                        tracing::error!("Could not send updated history to UI: {error}");
                    }
                }
                if let Err(error) = self.save_history() {
                    tracing::error!("Could not save updated history: {error}");
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
            let listener = TcpListener::bind(format!("127.0.0.1:{UI_LISTENING_PORT}")).context(
                format!("UI listener could not bind to \"127.0.0.1:{UI_SENDING_PORT}\"."),
            )?;

            let mut get_stream_consecutive_failures = 0;
            for stream in listener.incoming() {
                let stream_success_result = (|| -> Result<()> {
                    let mut stream =
                        stream.context("Could not get stream from incoming UI connexion.")?;
                    let mut reader = BufReader::new(&mut stream);
                    let mut request = String::new();
                    reader
                        .by_ref()
                        .take(64)
                        .read_line(&mut request)
                        .context("Could not read the incoming request from the UI.")?;
                    if !request.ends_with('\n') {
                        return Err(anyhow!("Invalid UI command."));
                    }

                    if request.trim() == "GET_HISTORY" {
                        drop(reader);
                        clippo
                            .send_history(stream.try_clone()?)
                            .context("Could not send the history to UI, stream.write() failed.")?;

                        tracing::info!(
                            "\"GET_HISTORY\" request received, sending current history to UI ..."
                        );
                    } else if request.trim() == "RESET_HISTORY" {
                        drop(reader);
                        clippo
                            .clear_history()
                            .context("Could not clear history after UI request.")?;

                        stream.write(b"OK")?;

                        tracing::info!(
                            "\"RESET_HISTORY\" request received, clearing current history ..."
                        );
                    } else if request.trim() == "GET_CAPTURE_STATE" {
                        drop(reader);
                        let paused = clippo
                            .capture_state
                            .lock()
                            .map_err(|e| anyhow!("Could not acquire capture state lock: {e}"))?
                            .paused;
                        stream.write_all(if paused { b"PAUSED" } else { b"RUNNING" })?;
                    } else if request.trim() == "PAUSE_CAPTURE"
                        || request.trim() == "RESUME_CAPTURE"
                    {
                        drop(reader);
                        clippo
                            .capture_state
                            .lock()
                            .map_err(|e| anyhow!("Could not acquire capture state lock: {e}"))?
                            .set_paused(request.trim() == "PAUSE_CAPTURE");
                        stream.write_all(b"OK")?;
                    } else if request.trim() == "DELETE_ENTRY" {
                        let mut payload = String::new();
                        reader.read_to_string(&mut payload)?;
                        drop(reader);
                        let entry: ClipboardHistoryEntry = ron::de::from_str(&payload)
                            .context("Could not deserialize entry to delete.")?;
                        stream.write_all(if clippo.delete_entry(&entry)? {
                            b"OK"
                        } else {
                            b"NOT_FOUND"
                        })?;
                    } else {
                        drop(reader);
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
        let history = self
            .history
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {e}"))?;
        Self::write_history(&history, Path::new(HISTORY_FILE_PATH))
    }

    fn write_history(history: &[ClipboardHistoryEntry], path: &Path) -> Result<()> {
        let serialized_history = ron::ser::to_string(history)
            .context("Could not serialize history when saving to file.")?;
        let temporary_path = path.with_extension("tmp");
        let mut file = fs::File::create(&temporary_path)
            .context(format!("Could not create {}", temporary_path.display()))?;

        file.write_all(serialized_history.as_bytes())
            .context(format!(
                "Could not write serialized history to {}",
                temporary_path.display()
            ))?;
        file.sync_all()
            .context(format!("Could not sync {}", temporary_path.display()))?;
        fs::rename(&temporary_path, path)
            .context(format!("Could not replace {}", path.display()))?;

        Ok(())
    }

    fn delete_entry(&self, entry: &ClipboardHistoryEntry) -> Result<bool> {
        let mut capture_state = self
            .capture_state
            .lock()
            .map_err(|e| anyhow!("Could not acquire capture state lock: {e}"))?;
        let mut history = self
            .history
            .lock()
            .map_err(|e| anyhow!("Could not acquire history lock: {e}"))?;
        let deleted =
            Self::delete_history_entry(&mut history, entry, Path::new(HISTORY_FILE_PATH))?;
        if deleted {
            capture_state.suppressed_entry = Some(entry.clone());
        }
        Ok(deleted)
    }

    fn delete_history_entry(
        history: &mut Vec<ClipboardHistoryEntry>,
        entry: &ClipboardHistoryEntry,
        path: &Path,
    ) -> Result<bool> {
        let Some(index) = history.iter().position(|saved| saved == entry) else {
            return Ok(false);
        };
        let removed = history.remove(index);
        if let Err(error) = Self::write_history(history, path) {
            history.insert(index, removed);
            return Err(error);
        }
        Ok(true)
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

#[cfg(test)]
mod tests {
    use super::{CaptureState, ClipboardHistoryEntry, Clippo};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resuming_capture_skips_the_current_clipboard_value() {
        let mut state = CaptureState::default();
        state.set_paused(true);
        state.set_paused(false);
        assert!(!state.paused);
        assert!(state.skip_next_read);
    }

    #[test]
    fn deleted_current_entry_stays_suppressed_until_clipboard_changes() {
        let secret = ClipboardHistoryEntry::Text("secret".into());
        let other = ClipboardHistoryEntry::Text("other".into());
        let mut state = CaptureState {
            suppressed_entry: Some(secret.clone()),
            ..Default::default()
        };
        assert!(!state.should_capture(&secret));
        assert!(state.should_capture(&other));
        assert!(state.should_capture(&secret));
    }

    #[test]
    fn deleting_one_entry_updates_memory_and_disk() {
        let path = std::env::temp_dir().join(format!(
            "clippo-delete-{}-{}.ron",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let keep = ClipboardHistoryEntry::Text("keep".into());
        let remove = ClipboardHistoryEntry::Text("remove".into());
        let mut history = vec![keep.clone(), remove.clone()];
        Clippo::write_history(&history, &path).unwrap();

        assert!(Clippo::delete_history_entry(&mut history, &remove, &path).unwrap());
        assert_eq!(history, vec![keep.clone()]);
        let saved: Vec<ClipboardHistoryEntry> =
            ron::de::from_reader(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(saved, vec![keep]);
        fs::remove_file(path).unwrap();
    }
}

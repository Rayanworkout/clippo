## Clippo - Clipboard Manager

**Clippo** is a powerful, cross-platform clipboard manager designed to help you easily retrieve and manage past clipboard entries. With a sleek minimal user interface and a background daemon, Clippo ensures that your clipboard history is always accessible.

Whether you're a developer, writer, or power user, Clippo simplifies your workflow by keeping track of everything you copy.

![Clippo Screenshot](./screenshot.png)

---

## Features

- **Clipboard History**: Access up to 100 previously copied items with ease.
- **Cross-Platform**: Works on Windows, macOS, and Linux.
- **Customizable Settings**:
  - Set a maximum display length for clipboard entries.
  - Minimize the UI automatically after copying or clearing.
  - Toggle between light and dark mode.
- **Daemon Support**: Runs in the background to track clipboard changes.
- **Easy Installation**: Install via Cargo or use the provided Linux install script.

---

![Clippo Screenshot 2](./screenshot2.png)

## Installation

### Prerequisites

_You need to have `Rust` and `Cargo` installed on your machine to run this tool. Official installation steps [here.](https://www.rust-lang.org/tools/install)_

### Linux

```bash
git clone https://github.com/Rayanworkout/clippo.git

cd clippo

cargo build --release

chmod u+x ./linux_install.sh

./linux_install.sh
```

This installs Clippo as a **user service** (`systemd --user`) and starts the daemon.

Useful commands:

```bash
systemctl --user status clippo_daemon.service
clippo_ui
```

## Architecture (Daemon + UI)

Clippo uses **two binaries by design**:

- `daemon` (`src/bin/daemon`)
- `ui` (`src/bin/ui`)

Why two binaries:

- The daemon is long-lived and keeps collecting clipboard history in the background.
- The UI is short-lived and can be opened/closed on demand without losing history.

### Responsibilities

- `daemon`
  - Polls the system clipboard.
  - Deduplicates and stores entries.
  - Persists history to `.clipboard_history.ron`.
  - Serves history to UI and handles reset commands.

- `ui`
  - Displays history and preferences.
  - Requests initial history on startup.
  - Receives live history updates from daemon.
  - Sends actions (ex: clear history) back to daemon.

### Local IPC Contract

Communication is local TCP on `127.0.0.1`:

- `7879`: daemon listens for UI requests (`GET_HISTORY`, `RESET_HISTORY`).
- `7878`: UI listens for daemon push updates (updated history payload).

This split keeps the UI simple while the daemon remains the source of truth.

## Local Development

Run in two terminals from repo root:

```bash
# Terminal 1
cargo run --bin daemon

# Terminal 2
cargo run --bin ui
```

Notes:

- If the daemon is not running, UI starts with empty/fallback history.
- The history file path is relative to daemon working directory.
  - In local dev, this is usually the repo root.
  - With Linux user service install, it is `~/.local/share/clippo/.clipboard_history.ron`.

## Contributing / Pull Requests

If you want to open a PR:

1. Fork and clone the repo.
2. Create a branch: `git checkout -b feat/your-change`.
3. Make your changes.
4. Validate locally:
   - `cargo fmt`
   - `cargo check --bin daemon`
   - `cargo check --bin ui`
5. Run both binaries manually (`daemon` + `ui`) and verify behavior.
6. Open the PR with:
   - What changed.
   - Why it changed.
   - Screenshots for UI changes (if relevant).

Design guideline:

- Keep daemon and UI loosely coupled through the current localhost protocol.
- Avoid tying daemon lifecycle to UI lifecycle (daemon should keep running independently).

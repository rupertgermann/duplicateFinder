# Duplicate Image Finder

A native macOS desktop application for finding and managing duplicate images, built with Tauri v2 (Rust backend, HTML/CSS/JS frontend).

![Rust](https://img.shields.io/badge/Rust-stable-orange) ![Tauri](https://img.shields.io/badge/Tauri-v2-blue) ![Platform](https://img.shields.io/badge/Platform-macOS-green) ![License](https://img.shields.io/badge/License-MIT-yellow)

## Screenshots

### Welcome Screen
![Welcome Screen](screenshots/welcome.png)

### Duplicate Groups View
![Main View](screenshots/main-view.png)

## Features

- **Fast parallel MD5 hashing** -- uses [rayon](https://crates.io/crates/rayon) for multithreaded hash computation across all CPU cores
- **Recursive folder scanning** -- walks the entire directory tree, following symlinks
- **Thumbnail preview** -- generates 250x250 thumbnails for side-by-side comparison
- **File details** -- dimensions, file size, and modification date for each image
- **Safe deletion** -- move files to trash (recoverable) or delete permanently, with confirmation dialogs
- **Smart selection helpers** -- bulk-select duplicates while keeping the first or largest file
- **Open file** -- open an image in your default viewer
- **Reveal in Folder** -- show the file in Finder
- **Group navigation** -- browse duplicate groups in a sidebar with prev/next controls
- **Cancel anytime** -- long scans can be cancelled mid-progress
- **Standalone native app** -- produces a ~12 MB macOS `.app` bundle with no runtime dependencies

## Supported Image Formats

JPG, JPEG, PNG, GIF, BMP, TIFF, TIF, WebP, HEIC, HEIF, AVIF, ICO

## Prerequisites

- [Rust toolchain](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) and npm

## Getting Started

```bash
# Clone this repository
git clone https://github.com/yourusername/duplicate-image-finder.git
cd duplicate-image-finder

# Install frontend dependencies
npm install

# Run in development mode (hot-reload)
npx tauri dev
```

## Building

```bash
npx tauri build
```

This build produces the release executable at:

```
src-tauri/target/release/duplicate-finder
```

To build a macOS `.app` bundle, run:

```bash
npx tauri build --bundles app
```

The app bundle is then created at:

```
src-tauri/target/release/bundle/macos/DuplicateImageFinder.app
```

## Usage

1. Click **Choose Folder** and select a directory to scan
2. Click **Scan** -- the app recursively finds all images and groups exact duplicates by MD5 hash
3. Browse duplicate groups in the left panel
4. For each group, review the thumbnails, file paths, sizes, and dates
5. Use the selection helpers:
   - **Select All Except First** -- keeps the first file, selects the rest
   - **Select All Except Largest** -- keeps the largest copy
   - **Deselect All** -- clear selection
6. Choose an action:
   - **Move to Trash** -- safe, recoverable via macOS Trash
   - **Delete Permanently** -- irreversible, requires extra confirmation
7. The group auto-updates or disappears once duplicates are resolved

## Tech Stack

| Component | Technology |
|-----------|------------|
| Backend | Rust (Tauri v2) |
| Frontend | HTML / CSS / vanilla JavaScript |
| Hashing | MD5 with parallel processing ([rayon](https://crates.io/crates/rayon)) |
| File walking | [walkdir](https://crates.io/crates/walkdir) |
| Image processing | [image](https://crates.io/crates/image) (thumbnails, dimensions) |
| Trash support | [trash](https://crates.io/crates/trash) |
| IPC | Tauri command system (Rust <-> JS) |

## Technical Architecture

The repository currently contains **two implementations** of the duplicate finder:

- **Active application** -- a Tauri v2 desktop app using a Rust backend and a single-file HTML/CSS/JavaScript frontend.
- **Legacy prototype** -- a standalone Python/Tkinter application in `duplicate_finder.py`.

Only the **Tauri app** is wired into the current `npm` and Tauri build flow.

## Runtime Flow

1. The desktop app starts in `src-tauri/src/main.rs`.
2. `main.rs` calls `duplicate_finder_lib::run()` from `src-tauri/src/lib.rs`.
3. `run()` creates the Tauri application, registers plugins, creates shared app state, and exposes Rust commands to the frontend.
4. Tauri serves `src/index.html` as the frontend because `src-tauri/tauri.conf.json` points `frontendDist` to `../src`.
5. The frontend uses `window.__TAURI__.core.invoke(...)` to call Rust commands such as `scan_folder`, `get_thumbnail`, `trash_files`, `delete_files`, `open_file`, and `reveal_file`.
6. The Rust backend emits `scan-progress` events while scanning, and the frontend listens to those events to update the status text and progress bar.

## File-by-File Responsibilities

### Root level

#### `README.md`
Project overview, usage instructions, screenshots, build information, and technical notes.

#### `package.json`
Node/Tauri entry for development and builds.

Responsibilities:

- Defines the app name and version for the JavaScript/Tauri toolchain.
- Provides the main scripts:
  - `npm run tauri`
  - `npm run dev`
  - `npm run build`
- Declares the Tauri JavaScript API and CLI dependencies.

#### `package-lock.json`
Exact npm dependency lockfile for reproducible installs.

#### `duplicate_finder.py`
Legacy Python desktop application built with Tkinter.

Responsibilities:

- Contains a complete older implementation of the duplicate finder.
- Handles folder scanning, MD5 hashing, thumbnail loading, group navigation, and delete/trash actions in Python.
- Uses `Pillow` for image handling and `send2trash` for recoverable deletes.

Current status:

- This file is **not referenced** by `package.json`, Tauri config, or the Rust app entry point.
- It appears to be a previous prototype or fallback implementation kept in the repository for reference.

#### `requirements.txt`
Python dependencies for the legacy Tkinter app.

Currently used only by `duplicate_finder.py`.

#### `.gitignore`
Git ignore rules for generated files and local artifacts.

#### `LICENSE`
Project license file.

### Frontend: `src/`

#### `src/index.html`
The entire active frontend lives in this single file.

It contains three concerns in one place:

- **HTML structure** -- toolbar, left duplicate-group list, right image-detail area, action bar, and status bar.
- **CSS styling** -- desktop-style layout and controls.
- **JavaScript application logic** -- state management, Tauri IPC calls, UI rendering, selection helpers, destructive actions, and progress updates.

Key responsibilities inside this file:

- Stores frontend state:
  - `groups`
  - `currentGroupIndex`
  - `scanning`
  - `cancelled`
  - `thumbnailCache`
- Opens the folder picker through the Tauri dialog plugin.
- Starts scans by invoking `scan_folder`.
- Listens for backend `scan-progress` events.
- Renders duplicate groups in the sidebar.
- Renders file rows with checkboxes, thumbnails, metadata, and per-file actions.
- Loads thumbnails lazily through the `get_thumbnail` Rust command.
- Supports bulk selection helpers:
  - select all except first
  - select all except largest
  - deselect all
- Calls `trash_files` and `delete_files` for destructive actions.
- Updates the in-memory UI state after files are removed so the current group or group list refreshes immediately.

Important note about the current implementation:

- The cancel button calls `invoke('cancel_scan')`, but there is **no `cancel_scan` command registered in Rust**.
- This means the UI has a cancel affordance, but backend cancellation is not currently implemented in the active Tauri app.

### Tauri backend: `src-tauri/`

#### `src-tauri/tauri.conf.json`
Main Tauri application configuration.

Responsibilities:

- Sets application identity:
  - product name
  - version
  - bundle identifier
- Defines the app window title and size constraints.
- Points the frontend distribution to `../src`.
- Enables global Tauri access in the frontend.
- Configures the content security policy.
- Configures the filesystem plugin behavior.

#### `src-tauri/Cargo.toml`
Rust package manifest for the active desktop application.

Responsibilities:

- Declares the crate name, version, and edition.
- Configures the library target used by Tauri.
- Declares Rust dependencies for:
  - Tauri core
  - dialog/opener/fs plugins
  - serialization
  - hashing
  - parallelism
  - file walking
  - image decoding and thumbnail generation
  - trash support

#### `src-tauri/Cargo.lock`
Rust lockfile for reproducible dependency resolution.

#### `src-tauri/build.rs`
Minimal Tauri build script.

Responsibility:

- Runs `tauri_build::build()` during compilation so Tauri can generate required build-time artifacts.

#### `src-tauri/src/main.rs`
Native executable entry point.

Responsibilities:

- Provides the desktop app `main()` function.
- Delegates application startup to `duplicate_finder_lib::run()`.
- Applies the standard Windows subsystem attribute to hide the extra console window in release mode.

#### `src-tauri/src/lib.rs`
This is the **core of the active application**.

It owns:

- shared data structures exchanged with the frontend
- duplicate scanning logic
- metadata extraction
- thumbnail generation
- file operations
- Tauri command registration
- application startup wiring

Key sections in `lib.rs`:

- **Data types**
  - `FileInfo` -- one file's path, name, size, modification time, and dimensions.
  - `DuplicateGroup` -- one duplicate set containing multiple `FileInfo` entries.
  - `ScanResult` -- wrapper returned to the frontend.
  - `ActionResult` -- success/error reporting for delete or trash operations.
  - `ScanProgress` -- payload emitted to the UI during scanning.
- **App state**
  - `AppState` stores the latest duplicate groups in shared Tauri state.
- **Helpers**
  - `is_image_file()` filters supported image extensions.
  - `format_size()` formats byte counts for display.
  - `format_modified()` and `days_to_ymd()` convert filesystem timestamps into readable UTC text without adding a chrono dependency.
  - `compute_md5()` hashes file contents in 64 KB chunks.
  - `build_file_info()` gathers metadata and image dimensions for the UI.
- **Tauri commands**
  - `scan_folder()` recursively walks the selected folder, filters image files, computes hashes in parallel with `rayon`, groups files by hash, builds `FileInfo` records, emits progress events, stores results in app state, and returns them to the frontend.
  - `get_file_info()` returns metadata for a single file.
  - `get_thumbnail()` loads an image, creates a `250x250` thumbnail, encodes it as PNG, and returns it as a base64 data URL for the frontend.
  - `trash_files()` sends files to the OS trash.
  - `delete_files()` removes files permanently.
  - `open_file()` opens a file in the default OS handler.
  - `reveal_file()` reveals the file in Finder/Explorer or opens the parent folder on Linux.
- **Application bootstrap**
  - `run()` builds the Tauri app, registers plugins, registers commands, initializes shared state, and starts the event loop.

### Assets and generated output

#### `screenshots/`
Documentation assets used by the README.

#### `src-tauri/icons/`
Application icons used for packaging/bundling.

#### `src-tauri/gen/`
Generated Tauri schema/build metadata.

#### `src-tauri/target/`
Rust build output directory.

#### `node_modules/`
Installed JavaScript dependencies.

## Duplicate Detection Logic

The active Tauri app detects **exact duplicates**, not visually similar images.

Current algorithm:

1. Recursively walk the selected directory.
2. Keep only files with supported image extensions.
3. Compute an MD5 hash for each image file.
4. Group files with identical hashes.
5. Discard groups with fewer than 2 files.
6. Build metadata for display and return the groups to the frontend.

Implications:

- Same pixels saved with different metadata or encoding may **not** match.
- The app is optimized for **byte-identical files**.
- Parallel hashing is the main performance optimization in the current implementation.

## Project Structure

``` 
duplicate-image-finder/
├── src/
│   └── index.html              # Active frontend: HTML, CSS, and JS in one file
├── src-tauri/
│   ├── Cargo.toml              # Rust package manifest and dependencies
│   ├── Cargo.lock              # Rust dependency lockfile
│   ├── build.rs                # Tauri build script
│   ├── tauri.conf.json         # Tauri app/window/frontend configuration
│   ├── icons/                  # App icons for packaging
│   ├── gen/                    # Generated Tauri files
│   ├── src/
│   │   ├── lib.rs              # Active backend logic and Tauri command definitions
│   │   └── main.rs             # Native executable entry point
│   └── target/                 # Rust build output
├── screenshots/                # README screenshots
├── duplicate_finder.py         # Legacy Python/Tkinter implementation
├── requirements.txt            # Python deps for legacy app
├── package.json                # npm scripts and Tauri JS dependencies
├── package-lock.json           # npm lockfile
├── LICENSE                     # MIT License
└── README.md
```

## Current State Summary

- **Production path**: Tauri app (`src/index.html` + `src-tauri/src/lib.rs`)
- **Legacy path**: Python/Tkinter app (`duplicate_finder.py`)
- **Frontend architecture**: single-file vanilla HTML/CSS/JS
- **Backend architecture**: Rust Tauri command backend
- **Duplicate strategy**: exact-file matching via MD5
- **Known mismatch**: frontend tries to call `cancel_scan`, but that command does not exist in the Rust backend

## License

[MIT](LICENSE)

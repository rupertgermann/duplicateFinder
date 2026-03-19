# Duplicate Image Finder

A Tauri v2 desktop application for finding and managing exact duplicate image files with a Rust backend and a single-file HTML/CSS/JavaScript frontend.

![Rust](https://img.shields.io/badge/Rust-stable-orange) ![Tauri](https://img.shields.io/badge/Tauri-v2-blue) ![App](https://img.shields.io/badge/App-DuplicateImageFinder-green) ![License](https://img.shields.io/badge/License-MIT-yellow)

## Screenshots

### Welcome Screen
![Welcome Screen](screenshots/welcome.png)

### Duplicate Groups View
![Main View](screenshots/main-view.png)

## Features

- Fast parallel MD5 hashing across all CPU cores with [rayon](https://crates.io/crates/rayon)
- Recursive folder scanning with symlink traversal via [walkdir](https://crates.io/crates/walkdir)
- Duplicate grouping for exact file matches
- Sidebar navigation for duplicate groups with previous/next controls
- Thumbnail previews rendered at `250x250`
- Per-file metadata including name, full path, dimensions, file size, raw byte size, and modification timestamp
- Bulk selection helpers for keeping the first file or the largest file in each group
- Per-file actions for opening a file or revealing it in the system file manager
- Group-level actions for moving selected files to Trash or deleting them permanently
- Progress updates during scan, hash, grouping, and metadata phases
- In-memory thumbnail caching for the active scan results
- Native release binary output plus macOS `.app` bundle support

## Supported Files

The scanner treats the following extensions as image files:

`jpg`, `jpeg`, `png`, `gif`, `bmp`, `tiff`, `tif`, `webp`, `heic`, `heif`, `avif`, `ico`

Thumbnail generation and image dimension extraction use the Rust [`image`](https://crates.io/crates/image) crate with decoders enabled for:

`jpeg`, `png`, `gif`, `bmp`, `tiff`, `webp`, `ico`

Files that match a scanned extension but cannot be decoded still participate in duplicate detection, but the UI can show `?` for dimensions or `[no preview]` for thumbnails.

## Requirements

- [Rust toolchain](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) and npm

## Setup

```bash
git clone https://github.com/rupertgermann/duplicateFinder.git
cd duplicateFinder
npm install
```

## Commands

| Command | Description |
| --- | --- |
| `npm run dev` | Start the Tauri app in development mode. |
| `npm run build` | Build the release binary with `tauri build`. |
| `npm run build -- --bundles app` | Build the macOS `.app` bundle. |
| `npm run tauri -- <args>` | Pass any additional Tauri CLI command or option through npm. |
| `npx tauri dev` | Direct Tauri CLI equivalent of `npm run dev`. |
| `npx tauri build` | Direct Tauri CLI equivalent of `npm run build`. |
| `npx tauri build --bundles app` | Direct Tauri CLI command for the macOS app bundle. |

## Build Output

`npm run build` produces the release executable at:

```text
src-tauri/target/release/duplicate-finder
```

`npm run build -- --bundles app` produces the macOS app bundle at:

```text
src-tauri/target/release/bundle/macos/DuplicateImageFinder.app
```

## Usage

1. Click `Choose Folder...` and select a directory to scan.
2. Click `Scan` to walk the directory tree, hash all matching image files, and group exact duplicates.
3. Browse duplicate groups in the left sidebar.
4. Review thumbnails, paths, dimensions, file sizes, and modification timestamps in the main panel.
5. Use `Select All Except First`, `Select All Except Largest`, or `Deselect All` to prepare a batch action.
6. Use `Move to Trash` for recoverable removal or `Delete Permanently` for direct file deletion.
7. Use `Open` or `Show in Folder` for per-file inspection before removing anything.
8. Continue through the remaining groups until the list is cleared.

## Current Behavior Notes

- Duplicate detection is based on exact MD5 matches, not visual similarity.
- Duplicate groups are sorted by group size first, then by the first file path in the group.
- Files inside each group are sorted by path.
- The frontend resets its state immediately after successful trash/delete actions so resolved groups disappear without a rescan.
- The toolbar includes a `Cancel` button. The current UI marks the scan as cancelling and ignores the eventual result, but there is no backend `cancel_scan` command, so the underlying scan job continues running until completion.

## Architecture

The active application is split into two main parts:

- `src/index.html`: the entire frontend, including layout, styling, event handling, rendering, selection helpers, and Tauri IPC calls
- `src-tauri/src/lib.rs`: the Rust backend for scanning, hashing, metadata extraction, thumbnail generation, file operations, progress events, and Tauri startup

The frontend calls these Tauri commands:

- `scan_folder`
- `get_file_info`
- `get_thumbnail`
- `trash_files`
- `delete_files`
- `open_file`
- `reveal_file`

The backend emits `scan-progress` events during long-running scans so the status bar and progress bar stay updated.

Detailed implementation notes live in [docs/technical-architecture.md](docs/technical-architecture.md).

## Project Structure

```text
duplicateFinder/
├── docs/
│   └── technical-architecture.md
├── screenshots/
│   ├── main-view.png
│   └── welcome.png
├── src/
│   └── index.html
├── src-tauri/
│   ├── capabilities/
│   │   └── default.json
│   ├── gen/
│   ├── icons/
│   ├── src/
│   │   ├── lib.rs
│   │   └── main.rs
│   ├── build.rs
│   ├── Cargo.lock
│   ├── Cargo.toml
│   └── tauri.conf.json
├── LICENSE
├── README.md
├── package-lock.json
└── package.json
```

## License

[MIT](LICENSE)

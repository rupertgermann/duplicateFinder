# Technical Architecture

## Overview

Duplicate Image Finder is a Tauri v2 desktop application with:

- a Rust backend in `src-tauri/src/lib.rs`
- a native entry point in `src-tauri/src/main.rs`
- a single-file frontend in `src/index.html`

The Tauri configuration in `src-tauri/tauri.conf.json` points `frontendDist` directly at `../src`, so the app loads the checked-in frontend file without a separate web build step.

## Runtime Flow

1. The native executable starts in `src-tauri/src/main.rs`.
2. `main()` calls `duplicate_finder_lib::run()`.
3. `run()` creates the Tauri application, registers plugins, initializes shared state, and exposes Rust commands to the frontend.
4. Tauri loads `src/index.html` as the frontend.
5. The frontend calls Rust commands through `window.__TAURI__.core.invoke(...)`.
6. The backend emits `scan-progress` events while scanning so the UI can update the status text and progress bar.

## Frontend Responsibilities

`src/index.html` contains all frontend concerns in one file:

- HTML structure for the toolbar, duplicate-group sidebar, image detail panel, action bar, and status bar
- CSS styling for the desktop-style interface
- JavaScript state management, rendering logic, IPC helpers, selection helpers, destructive actions, and progress handling

### Frontend State

The frontend keeps these main pieces of state in memory:

- `groups`
- `currentGroupIndex`
- `scanning`
- `cancelled`
- `thumbnailCache`

### Frontend Behaviors

- Opens the folder picker with the Tauri dialog API
- Starts scans with `scan_folder`
- Cancels scans with `cancel_scan`
- Listens for `scan-progress`
- Renders duplicate groups in the sidebar
- Renders each file row with a checkbox, thumbnail, metadata, and per-file actions
- Loads thumbnails lazily with `get_thumbnail`
- Applies selection helpers for first-file and largest-file retention strategies
- Calls `trash_files` and `delete_files` for batch actions
- Removes deleted files from the in-memory state immediately so resolved groups disappear without a full rescan

## Backend Responsibilities

`src-tauri/src/lib.rs` owns:

- shared data structures exchanged with the frontend
- duplicate detection logic
- metadata extraction
- thumbnail generation
- file operations
- Tauri command registration
- application startup wiring

### Data Structures

| Type | Purpose |
| --- | --- |
| `FileInfo` | Path, file name, formatted size, raw byte size, modified timestamp, and dimensions for one file |
| `DuplicateGroup` | One duplicate set containing multiple `FileInfo` records |
| `ScanResult` | Wrapper returned from `scan_folder` |
| `ActionResult` | Success count and per-file errors for trash/delete operations |
| `ScanProgress` | Progress event payload with a user-facing message and fractional completion |
| `AppState` | Shared application state containing the latest duplicate groups and scan-cancellation flag |

### Helper Functions

| Function | Purpose |
| --- | --- |
| `is_image_file()` | Filters files by supported extension |
| `format_size()` | Formats byte counts for display |
| `format_modified()` | Formats filesystem timestamps into `YYYY-MM-DD HH:MM` text |
| `days_to_ymd()` | Supports timestamp formatting without adding a date/time crate |
| `is_scan_cancelled()` | Reads the shared cooperative cancellation flag |
| `compute_md5()` | Streams file contents in `64 KB` chunks, checks for cancellation, and computes the MD5 hash |
| `build_file_info()` | Reads metadata and image dimensions for the UI |

### Tauri Commands

| Command | Purpose | Used by current UI |
| --- | --- | --- |
| `scan_folder(folder)` | Walk the selected directory, hash image files in parallel, group exact duplicates, build `FileInfo` output, emit progress, and store results in app state | Yes |
| `cancel_scan()` | Request cooperative cancellation of the active scan job | Yes |
| `get_file_info(path)` | Return metadata for a single file | No |
| `get_thumbnail(path)` | Generate a `250x250` PNG thumbnail and return it as a base64 data URL | Yes |
| `trash_files(paths)` | Send selected files to the operating system trash | Yes |
| `delete_files(paths)` | Permanently remove selected files from disk | Yes |
| `open_file(path)` | Open a file with the default operating system handler | Yes |
| `reveal_file(path)` | Reveal the file in Finder or Explorer, or open the parent directory on Linux | Yes |

## Duplicate Detection Pipeline

The application detects exact duplicate files rather than visually similar images.

1. Walk the selected directory recursively with `WalkDir`.
2. Follow symlinks during traversal.
3. Keep only file entries with recognized image extensions.
4. Compute MD5 hashes for those files in parallel with `rayon`.
5. Group files by identical hash value.
6. Discard groups with fewer than two files.
7. Build `FileInfo` records for the remaining files.
8. Sort groups by descending group size, then by first file path.
9. Sort files inside each group by path.

Cancellation is cooperative: the backend checks a shared atomic flag during directory walking, while reading file bytes for hashing, between parallel hash tasks, and while assembling output metadata. That lets the current safe unit of work finish and then exits without publishing partial scan results.

## Progress Events

The backend emits `scan-progress` events with a `ScanProgress` payload during these phases:

- initial file discovery
- hash computation
- duplicate grouping
- file metadata assembly
- final completion

Hashing progress emits every 50 files, or at the final file, to avoid flooding the event channel.

## File Format Handling

The scanner recognizes these extensions:

`jpg`, `jpeg`, `png`, `gif`, `bmp`, `tiff`, `tif`, `webp`, `heic`, `heif`, `avif`, `ico`

Thumbnail generation and image dimension extraction use the `image` crate with decoding support enabled for:

`jpeg`, `png`, `gif`, `bmp`, `tiff`, `webp`, `ico`

That means a file can still be scanned and grouped as a duplicate even when the frontend cannot render a thumbnail or dimensions for it.

## Desktop Integration

The backend handles file opening and reveal actions per platform:

- macOS: `open` and `open -R`
- Windows: `explorer` and `explorer /select,`
- Linux: `xdg-open` for files or parent directories

The native entry point also sets the standard Windows release-mode subsystem flag so packaged builds do not show an extra console window.

## Tauri Configuration

`src-tauri/tauri.conf.json` defines:

- `productName`: `DuplicateImageFinder`
- `identifier`: `com.duplicatefinder.desktop`
- `frontendDist`: `../src`
- `withGlobalTauri`: `true`
- a single main window titled `DuplicateImageFinder` with size `1100x720` and minimum size `900x550`
- a content security policy that allows local assets and data URLs for image previews
- filesystem plugin configuration with `requireLiteralLeadingDot: false`

## Capabilities and Permissions

`src-tauri/capabilities/default.json` grants the main window:

- `core:default`
- `dialog:default`
- `dialog:allow-open`
- `fs:default`
- `opener:default`

The application also registers these Tauri plugins during startup:

- `tauri-plugin-dialog`
- `tauri-plugin-opener`
- `tauri-plugin-fs`

## Build and Packaging

The repository uses npm only as a lightweight wrapper around the Tauri CLI:

| Script | Underlying command |
| --- | --- |
| `npm run tauri` | `tauri` |
| `npm run dev` | `tauri dev` |
| `npm run build` | `tauri build` |

Important output paths:

- Release binary: `src-tauri/target/release/duplicate-finder`
- macOS app bundle: `src-tauri/target/release/bundle/macos/DuplicateImageFinder.app`

## Current Behavior Notes

- Scan results are stored in shared Tauri state after `scan_folder()` completes.
- The frontend caches thumbnails for the active scan result in memory only.
- The `Cancel` button invokes `cancel_scan`, disables further cancel clicks, and ignores further progress events while the backend winds down.
- Cancelled scans do not overwrite the shared backend group state with partial or stale results.

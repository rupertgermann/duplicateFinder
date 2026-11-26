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
- `destructiveActionPending`
- `keeperId`
- `thumbnailCache`

### Frontend Behaviors

- Opens the folder picker with the Tauri dialog API
- Starts scans with `scan_folder` and reports root scan errors and partial scan counts in the status bar
- Cancels scans with `cancel_scan`
- Listens for `scan-progress`
- Renders duplicate groups in the sidebar
- Renders each file row with a checkbox, thumbnail, metadata, and per-file actions
- Loads thumbnails lazily with `get_thumbnail`
- Applies selection helpers for first-file and largest-file retention strategies
- Calls `trash_files` and `delete_files` for batch actions
- Removes files by the returned success IDs in the group that started the action and drops the group if fewer than two files remain

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
| `FileInfo` | Display path, file name, sizes, modified timestamp, and dimensions. Scan results also include a file ID. |
| `DuplicateGroup` | One duplicate set containing multiple `FileInfo` records |
| `ScanResult` | Duplicate `groups` and `skipped` counts for `entries`, `unreadable_files`, and `metadata_files` |
| `ActionResult` | `succeeded_ids` and a list of `failures` with each file's `id` and `error` |
| `ScanProgress` | Progress event payload with a user-facing message and fractional completion |
| `AppState` | Shared state containing IDs, native paths, and snapshots for the latest scan, plus active scan cancellation state |

### Helper Functions

| Function | Purpose |
| --- | --- |
| `is_image_file()` | Filters files by supported extension |
| `format_size()` | Formats byte counts for display |
| `format_modified()` | Formats filesystem timestamps into `YYYY-MM-DD HH:MM` text |
| `days_to_ymd()` | Supports timestamp formatting without adding a date/time crate |
| `check_cancelled()` | Checks the cooperative cancellation flag and returns an error if cancellation was requested |
| `compute_md5()` | Streams file contents in `64 KB` chunks, checks for cancellation, and computes the MD5 hash |
| `build_file_info()` | Formats file metadata and reads image dimensions for the UI |

### Tauri Commands

| Command | Purpose | Used by current UI |
| --- | --- | --- |
| `scan_folder(folder)` | Walk the selected directory, hash image files in parallel, group exact duplicates, build `FileInfo` output, emit progress, and store results in app state | Yes |
| `cancel_scan()` | Request cooperative cancellation of the active scan job | Yes |
| `get_file_info(path)` | Return metadata for a single file with `id` set to `null` | No |
| `get_thumbnail(id)` | Generate a `250x250` PNG thumbnail and return it as a base64 data URL | Yes |
| `trash_files(ids, keeperId, allowRemoveAll)` | Send selected files to the operating system trash | Yes |
| `delete_files(ids, keeperId, allowRemoveAll)` | Permanently remove selected files from disk | Yes |
| `open_file(id)` | Open a file with the default operating system handler | Yes |
| `reveal_file(id)` | Show the file in the system file manager | Yes |

The backend uses file IDs that are unique across scans to look up native paths. The frontend displays paths and names as text.

Before removing each file, the backend checks its path, filesystem identity, size, timestamps, and MD5 hash against the scan. An unchanged, unselected file must have the same bytes unless the user confirms removal of the entire group. When the frontend supplies a keeper ID, the backend checks that keeper.

## Duplicate Detection Pipeline

1. Walk the selected directory recursively with `WalkDir` and follow directory symlinks. Track directories by filesystem identity so each directory is visited once even if it has several paths. Fail the scan if the scan root is unavailable.
2. Find regular files with recognized image extensions and resolve their paths. The scanner excludes file symlinks and counts each hard link as a separate entry.
3. Compute MD5 hashes for those files in parallel with `rayon` and group files by hash.
4. Build `FileInfo` records for members of duplicate groups and discard groups with fewer than two usable records.
5. Sort files inside each group by path. Sort groups by descending group size, then by first file path.

The scan counts recoverable traversal, file read, and metadata errors as skipped items and returns the remaining duplicate groups.

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

The backend uses platform commands to open and reveal files.

- On macOS, `open` opens the file and `open -R` reveals it in Finder.
- On Windows, `explorer` opens the file and `explorer /select,` reveals it in File Explorer.
- On Linux, `xdg-open` opens the file or its parent directory.

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

- After a successful scan, the backend stores the corresponding file IDs and native paths in shared Tauri state.
- The frontend caches thumbnails by file ID for the active scan.
- The `Cancel` button invokes `cancel_scan`, disables further cancel clicks, and ignores further progress events while the backend winds down.
- Cancelled scans end without publishing duplicate groups.

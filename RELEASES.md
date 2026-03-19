# Release Notes - Duplicate Image Finder v1.0.1

## Overview

Bug fix and UX improvement release featuring cooperative scan cancellation and a complete visual refresh with dark mode support.

## What's New

### New Features
- **Cooperative scan cancellation** — The Cancel button now actually stops active scans. Cancellation is checked during file discovery, hashing, and metadata assembly phases, so scans stop as soon as in-flight work reaches a safe interruption point.
- **Dark mode theme** — Toggle between light and dark themes with a new theme switcher in the toolbar. The app respects the system preference by default and remembers your choice.

### UI Improvements
- **Transparent titlebar** — macOS app now features a modern transparent titlebar with a dark background (`#15151f`)
- **Complete visual refresh** — Updated color palette, improved typography with DM Sans and JetBrains Mono fonts, refined spacing and shadows
- **Better accessibility** — Focus-visible indicators and smoother theme transitions throughout the interface

### API Changes
- Added new Tauri command: `cancel_scan` — call to request cooperative cancellation of an ongoing scan

## Bug Fixes
- Fixed: Cancel button previously only marked the scan as cancelled in the UI while the backend job continued running until completion

## Known Limitations

- Duplicate detection is based on exact MD5 hash matches, not visual similarity
- Files that cannot be decoded show `?` for dimensions and `[no preview]` for thumbnails

---

# Release Notes - Duplicate Image Finder v1.0.0

## Overview

First stable release of Duplicate Image Finder — a fast, native desktop application for finding and managing exact duplicate image files.

## What's New

### Core Features
- **Fast parallel scanning** — Multi-threaded MD5 hashing across all CPU cores using Rayon
- **Recursive folder traversal** — Scans directories including symlinked paths via Walkdir
- **Duplicate grouping** — Automatically groups exact file matches for easy review
- **Progress tracking** — Real-time progress updates during scan, hash, grouping, and metadata phases

### User Interface
- **Sidebar navigation** — Browse duplicate groups with previous/next controls
- **Thumbnail previews** — 250x250 image previews with in-memory caching
- **Detailed metadata** — View file name, full path, dimensions, file size, raw bytes, and modification date
- **Bulk selection helpers** — Quick actions to keep first file or largest file in each group
- **Per-file actions** — Open file or reveal in system file manager
- **Group actions** — Move selected files to Trash or delete permanently

### Supported Formats
Scans: `jpg`, `jpeg`, `png`, `gif`, `bmp`, `tiff`, `tif`, `webp`, `heic`, `heif`, `avif`, `ico`

Thumbnails: `jpeg`, `png`, `gif`, `bmp`, `tiff`, `webp`, `ico`

### Distribution
- Native release binaries
- macOS `.app` bundle support

## System Requirements

- macOS (bundled app available)
- Rust toolchain for building from source
- Node.js and npm for development

## Installation

### macOS
Download and install `DuplicateImageFinder.app` from the releases page.

### Build from Source
```bash
git clone https://github.com/rupertgermann/duplicateFinder.git
cd duplicateFinder
npm install
npm run build
```

## Known Limitations

- Duplicate detection is based on exact MD5 hash matches, not visual similarity
- Cancel button marks scan as cancelling but underlying job continues until completion
- Files that cannot be decoded show `?` for dimensions and `[no preview]` for thumbnails

## Full Changelog

See the [GitHub repository](https://github.com/rupertgermann/duplicateFinder) for detailed commit history.

---

**Built with:** Tauri v2, Rust, HTML/CSS/JavaScript  
**License:** MIT

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

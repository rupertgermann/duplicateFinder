# Duplicate Image Finder

Cross-platform GUI tool for finding and cleaning up duplicate images on your hard disk.

![Python](https://img.shields.io/badge/Python-3.9+-blue) ![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-green) ![License](https://img.shields.io/badge/License-MIT-yellow)

## Screenshots

### Welcome Screen
![Welcome Screen](screenshots/welcome.png)

### Duplicate Groups View
![Main View](screenshots/main-view.png)

## Features

- **Fast MD5 scanning** — finds exact byte-identical duplicate images using multithreaded MD5 hashing
- **Thumbnail previews** — see what you're deleting before you delete it
- **File details** — dimensions, file size, and modification date for each image
- **Safe deletion** — move files to trash (recoverable) or delete permanently, always with confirmation dialogs
- **Smart selection** — bulk-select duplicates while keeping the first or largest file
- **Cross-platform** — native look and feel on macOS, Linux, and Windows (tkinter)
- **Cancel anytime** — long scans can be cancelled mid-progress

## Supported Formats

JPG, JPEG, PNG, GIF, BMP, TIFF, WebP, HEIC, HEIF, AVIF, ICO, SVG

## Installation

```bash
# Clone this repository
git clone https://github.com/yourusername/duplicate-image-finder.git
cd duplicate-image-finder

# Create a virtual environment and install dependencies
python3 -m venv venv
source venv/bin/activate        # macOS/Linux
# venv\Scripts\activate          # Windows

pip install -r requirements.txt
```

### Dependencies

| Package | Purpose |
|---------|---------|
| [Pillow](https://pypi.org/project/Pillow/) | Image loading, thumbnails, dimension detection |
| [send2trash](https://pypi.org/project/Send2Trash/) | Cross-platform "move to trash" |

> **Note:** The GUI uses Python's built-in `tkinter` — no additional GUI framework needed. On some Linux distros you may need to install it separately (e.g. `sudo apt install python3-tk`).

## Usage

```bash
source venv/bin/activate        # macOS/Linux
python3 duplicate_finder.py
```

### Workflow

1. Click **Choose Folder** and select a directory to scan
2. Click **Scan** — the app recursively finds all images and groups exact duplicates
3. Browse duplicate groups in the left panel
4. For each group, review the thumbnails, file paths, sizes, and dates
5. Use the selection helpers:
   - **Select All Except First** — keeps the first file, selects the rest
   - **Select All Except Largest** — keeps the largest copy
   - **Deselect All** — clear selection
6. Choose an action:
   - **Move Selected to Trash** — safe, recoverable via your OS trash
   - **Delete Selected Permanently** — irreversible, requires extra confirmation
7. The group auto-updates or disappears once duplicates are resolved

### Additional Controls

- **Open** — open an image in your default viewer
- **Show in Folder** — reveal the file in Finder / Explorer / file manager
- **Cancel** — stop a scan in progress
- **Prev / Next** — navigate between duplicate groups

## How It Works

The scanner walks the selected directory tree and collects all files with image extensions. It then computes an MD5 hash for each file using a thread pool. Files with identical MD5 hashes are grouped as exact duplicates.

The scan is fully **read-only** — no files are touched until you explicitly choose to trash or delete them, and every destructive action requires a confirmation dialog.

## Project Structure

```
duplicate-image-finder/
├── duplicate_finder.py   # Single-file application (engine + GUI)
├── requirements.txt      # Python dependencies
├── screenshots/           # App screenshots
├── LICENSE               # MIT License
└── README.md
```

## License

[MIT](LICENSE)

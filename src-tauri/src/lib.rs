use base64::Engine;
use md5::{Digest, Md5};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub size: String,
    pub size_bytes: u64,
    pub modified: String,
    pub dimensions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub files: Vec<FileInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub groups: Vec<DuplicateGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub succeeded: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub message: String,
    pub fraction: f64,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct AppState {
    pub groups: Vec<DuplicateGroup>,
    pub cancel_requested: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "heic", "heif", "avif", "ico",
];
const SCAN_CANCELLED_MESSAGE: &str = "Scan cancelled.";

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_modified(metadata: &fs::Metadata) -> String {
    match metadata.modified() {
        Ok(time) => {
            let duration = time
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = duration.as_secs() as i64;

            // Manual UTC conversion (no chrono dependency)
            let days = secs / 86400;
            let time_of_day = secs % 86400;
            let hours = time_of_day / 3600;
            let minutes = (time_of_day % 3600) / 60;

            // Days since 1970-01-01
            let (year, month, day) = days_to_ymd(days);
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                year, month, day, hours, minutes
            )
        }
        Err(_) => "?".to_string(),
    }
}

fn days_to_ymd(mut days: i64) -> (i64, i64, i64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month index [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as i64, d as i64)
}

fn is_scan_cancelled(cancel_requested: &AtomicBool) -> bool {
    cancel_requested.load(Ordering::Relaxed)
}

fn compute_md5(path: &Path, cancel_requested: &AtomicBool) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Cannot open {}: {}", path.display(), e))?;
    let mut hasher = Md5::new();
    let mut buffer = [0u8; 65536]; // 64 KB chunks
    loop {
        if is_scan_cancelled(cancel_requested) {
            return Err(SCAN_CANCELLED_MESSAGE.to_string());
        }

        let bytes_read = file
            .read(&mut buffer)
            .map_err(|e| format!("Read error {}: {}", path.display(), e))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

fn build_file_info(path: &Path) -> Result<FileInfo, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("Metadata error: {}", e))?;
    let size_bytes = metadata.len();

    let dimensions = match image::image_dimensions(path) {
        Ok((w, h)) => format!("{}x{}", w, h),
        Err(_) => "?".to_string(),
    };

    Ok(FileInfo {
        path: path.to_string_lossy().to_string(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        size: format_size(size_bytes),
        size_bytes,
        modified: format_modified(&metadata),
        dimensions,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn scan_folder(
    folder: String,
    app: AppHandle,
    state: tauri::State<'_, Mutex<AppState>>,
) -> Result<ScanResult, String> {
    let cancel_requested = {
        let mut app_state = state.lock().map_err(|e| format!("Lock error: {}", e))?;
        app_state.groups.clear();
        app_state.cancel_requested.store(false, Ordering::SeqCst);
        Arc::clone(&app_state.cancel_requested)
    };

    let result = tauri::async_runtime::spawn_blocking(move || {
        // Phase 1: collect image files
        let _ = app.emit(
            "scan-progress",
            ScanProgress {
                message: "Scanning for image files...".to_string(),
                fraction: 0.0,
            },
        );

        let mut image_files = Vec::new();
        for entry in WalkDir::new(&folder).follow_links(true).into_iter() {
            if is_scan_cancelled(&cancel_requested) {
                return Err::<ScanResult, String>(SCAN_CANCELLED_MESSAGE.to_string());
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            if entry.file_type().is_file() && is_image_file(entry.path()) {
                image_files.push(entry.path().to_string_lossy().to_string());
            }
        }

        let total = image_files.len();
        if total == 0 {
            return Ok::<ScanResult, String>(ScanResult { groups: vec![] });
        }

        let _ = app.emit(
            "scan-progress",
            ScanProgress {
                message: format!("Found {} image files. Computing hashes...", total),
                fraction: 0.05,
            },
        );

        // Phase 2: compute MD5 hashes in parallel
        let completed = AtomicUsize::new(0);
        let app_ref = &app;
        let completed_ref = &completed;
        let cancel_ref = &cancel_requested;

        let hashes: Vec<(String, String)> = image_files
            .par_iter()
            .try_fold(
                Vec::new,
                |mut acc, file_path| -> Result<Vec<(String, String)>, String> {
                    if is_scan_cancelled(cancel_ref) {
                        return Err(SCAN_CANCELLED_MESSAGE.to_string());
                    }

                    let path = Path::new(file_path);
                    let hash = match compute_md5(path, cancel_ref) {
                        Ok(hash) => hash,
                        Err(error) if error == SCAN_CANCELLED_MESSAGE => return Err(error),
                        Err(_) => return Ok(acc),
                    };

                    let done = completed_ref.fetch_add(1, Ordering::Relaxed) + 1;
                    // Emit progress every 50 files to avoid flooding
                    if done % 50 == 0 || done == total {
                        let fraction = 0.05 + 0.85 * (done as f64 / total as f64);
                        let _ = app_ref.emit(
                            "scan-progress",
                            ScanProgress {
                                message: format!("Hashing... {}/{}", done, total),
                                fraction,
                            },
                        );
                    }

                    acc.push((file_path.clone(), hash));
                    Ok(acc)
                },
            )
            .try_reduce(Vec::new, |mut left, mut right| {
                left.append(&mut right);
                Ok(left)
            })?;

        if is_scan_cancelled(&cancel_requested) {
            return Err::<ScanResult, String>(SCAN_CANCELLED_MESSAGE.to_string());
        }

        // Phase 3: group by hash
        let _ = app.emit(
            "scan-progress",
            ScanProgress {
                message: "Grouping duplicates...".to_string(),
                fraction: 0.92,
            },
        );

        let mut hash_map: HashMap<String, Vec<String>> = HashMap::new();
        for (path, hash) in hashes {
            if is_scan_cancelled(&cancel_requested) {
                return Err::<ScanResult, String>(SCAN_CANCELLED_MESSAGE.to_string());
            }
            hash_map.entry(hash).or_default().push(path);
        }

        // Keep only groups with 2+ files
        let mut groups: Vec<Vec<String>> = hash_map
            .into_values()
            .filter(|paths| paths.len() >= 2)
            .collect();

        // Sort groups: larger groups first, then by first filename
        groups.sort_by(|a, b| {
            b.len().cmp(&a.len()).then_with(|| {
                let a_first = a.first().map(|s| s.as_str()).unwrap_or("");
                let b_first = b.first().map(|s| s.as_str()).unwrap_or("");
                a_first.cmp(b_first)
            })
        });

        // Build FileInfo for each file in each group
        let _ = app.emit(
            "scan-progress",
            ScanProgress {
                message: "Building file info...".to_string(),
                fraction: 0.95,
            },
        );

        let mut duplicate_groups = Vec::new();
        for paths in groups {
            if is_scan_cancelled(&cancel_requested) {
                return Err::<ScanResult, String>(SCAN_CANCELLED_MESSAGE.to_string());
            }

            let mut files: Vec<FileInfo> = paths
                .iter()
                .filter_map(|p| build_file_info(Path::new(p)).ok())
                .collect();
            // Sort files within a group by path
            files.sort_by(|a, b| a.path.cmp(&b.path));

            if !files.is_empty() {
                duplicate_groups.push(DuplicateGroup { files });
            }
        }

        if is_scan_cancelled(&cancel_requested) {
            return Err::<ScanResult, String>(SCAN_CANCELLED_MESSAGE.to_string());
        }

        let _ = app.emit(
            "scan-progress",
            ScanProgress {
                message: format!("Done. Found {} duplicate groups.", duplicate_groups.len()),
                fraction: 1.0,
            },
        );

        Ok(ScanResult {
            groups: duplicate_groups,
        })
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))??;

    // Store results in app state
    {
        let mut app_state = state.lock().map_err(|e| format!("Lock error: {}", e))?;
        app_state.groups = result.groups.clone();
    }

    Ok(result)
}

#[tauri::command]
fn cancel_scan(state: tauri::State<'_, Mutex<AppState>>) -> Result<(), String> {
    let app_state = state.lock().map_err(|e| format!("Lock error: {}", e))?;
    app_state.cancel_requested.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
fn get_file_info(path: String) -> Result<FileInfo, String> {
    build_file_info(Path::new(&path))
}

#[tauri::command]
fn get_thumbnail(path: String) -> Result<String, String> {
    let img = image::open(&path).map_err(|e| format!("Cannot open image {}: {}", path, e))?;
    let thumbnail = img.thumbnail(250, 250);
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    thumbnail
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Encoding error: {}", e))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:image/png;base64,{}", b64))
}

#[tauri::command]
fn trash_files(paths: Vec<String>) -> Result<ActionResult, String> {
    let mut succeeded = 0usize;
    let mut errors = Vec::new();

    for path in &paths {
        match trash::delete(path) {
            Ok(_) => succeeded += 1,
            Err(e) => errors.push(format!("{}: {}", path, e)),
        }
    }

    Ok(ActionResult { succeeded, errors })
}

#[tauri::command]
fn delete_files(paths: Vec<String>) -> Result<ActionResult, String> {
    let mut succeeded = 0usize;
    let mut errors = Vec::new();

    for path in &paths {
        match fs::remove_file(path) {
            Ok(_) => succeeded += 1,
            Err(e) => errors.push(format!("{}: {}", path, e)),
        }
    }

    Ok(ActionResult { succeeded, errors })
}

#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
fn reveal_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| format!("Failed to reveal file: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .map_err(|e| format!("Failed to reveal file: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        // Try xdg-open on the parent directory
        let parent = Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        std::process::Command::new("xdg-open")
            .arg(&parent)
            .spawn()
            .map_err(|e| format!("Failed to reveal file: {}", e))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .manage(Mutex::new(AppState::default()))
        .invoke_handler(tauri::generate_handler![
            scan_folder,
            cancel_scan,
            get_file_info,
            get_thumbnail,
            trash_files,
            delete_files,
            open_file,
            reveal_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

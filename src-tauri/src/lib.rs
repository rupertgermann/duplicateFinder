use base64::Engine;
use md5::{Digest, Md5};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{self, BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub id: Option<String>,
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
    pub skipped: Skipped,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Skipped {
    pub entries: usize,
    pub unreadable_files: usize,
    pub metadata_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub succeeded_ids: Vec<String>,
    pub failures: Vec<ActionFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionFailure {
    pub id: String,
    pub error: String,
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
    scan_number: u64,
    cancel_requested: Option<Arc<AtomicBool>>,
    duplicates: ScanFiles,
}

#[derive(Debug, Default)]
struct ScanFiles {
    files: HashMap<String, ScannedFile>,
    groups: Vec<Vec<String>>,
}

#[derive(Debug)]
struct ScannedFile {
    snapshot: FileSnapshot,
    group: usize,
}

#[derive(Debug)]
struct FileSnapshot {
    path: PathBuf,
    parent_identity: FileIdentity,
    stamp: FileStamp,
    hash: [u8; 16],
}

type FileIdentity = (u64, u128);

#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    identity: FileIdentity,
    size: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
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

fn check_cancelled(cancel_requested: &AtomicBool) -> Result<(), String> {
    if cancel_requested.load(Ordering::Relaxed) {
        return Err(SCAN_CANCELLED_MESSAGE.to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn unix_file_identity(metadata: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    (metadata.dev(), metadata.ino().into())
}

#[cfg(windows)]
fn windows_file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
    };

    let handle = file.as_raw_handle();
    let mut info = FILE_ID_INFO::default();
    // The OS fills these structures for the valid handle borrowed from File.
    unsafe {
        if GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        ) != 0
        {
            return Ok((
                info.VolumeSerialNumber,
                u128::from_le_bytes(info.FileId.Identifier),
            ));
        }

        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if GetFileInformationByHandle(handle, &mut info) != 0 {
            let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
            return Ok((info.dwVolumeSerialNumber.into(), index.into()));
        }
    }
    Err(io::Error::last_os_error())
}

fn directory_identity(path: &Path) -> io::Result<FileIdentity> {
    #[cfg(unix)]
    let metadata = fs::metadata(path)?;
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        };

        File::options()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
    };
    #[cfg(windows)]
    let metadata = file.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::other("The parent is no longer a directory."));
    }
    #[cfg(unix)]
    return Ok(unix_file_identity(&metadata));
    #[cfg(windows)]
    return windows_file_identity(&file);
}

impl FileStamp {
    fn read(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("The entry is no longer a regular file."));
        }
        Ok(Self {
            #[cfg(unix)]
            identity: unix_file_identity(&metadata),
            #[cfg(windows)]
            identity: windows_file_identity(file)?,
            size: metadata.len(),
            modified: metadata.modified()?,
            created: metadata.created().ok(),
        })
    }
}

fn compute_md5(file: &mut File, cancel_requested: Option<&AtomicBool>) -> io::Result<[u8; 16]> {
    let mut hasher = Md5::new();
    let mut buffer = [0u8; 65536];
    loop {
        if cancel_requested.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                SCAN_CANCELLED_MESSAGE,
            ));
        }
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buffer[..bytes_read]);
    }
}

fn open_canonical_file(path: &Path) -> io::Result<File> {
    if !fs::symlink_metadata(path)?.is_file() || dunce::canonicalize(path)? != path {
        return Err(io::Error::other(
            "The entry no longer resolves to the scanned file.",
        ));
    }
    File::open(path)
}

impl FileSnapshot {
    fn open_checked(&self) -> io::Result<File> {
        let file = open_canonical_file(&self.path)?;
        let parent = self.path.parent().expect("scanned file has a parent");
        if directory_identity(parent)? != self.parent_identity
            || FileStamp::read(&file)? != self.stamp
        {
            return Err(io::Error::other(
                "The file changed since the scan. Scan again.",
            ));
        }
        Ok(file)
    }

    fn open_verified(&self) -> io::Result<File> {
        let mut file = self.open_checked()?;
        if compute_md5(&mut file, None)? != self.hash || FileStamp::read(&file)? != self.stamp {
            return Err(io::Error::other(
                "The file changed since the scan. Scan again.",
            ));
        }
        Ok(file)
    }

    fn same_entry(&self, other: &Self) -> bool {
        self.parent_identity == other.parent_identity
            && self.path.file_name() == other.path.file_name()
    }
}

fn build_file_info(path: &Path, metadata: &Metadata, id: Option<String>) -> FileInfo {
    let dimensions = match image::image_dimensions(path) {
        Ok((w, h)) => format!("{}x{}", w, h),
        Err(_) => "?".to_string(),
    };

    FileInfo {
        id,
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: format_size(metadata.len()),
        size_bytes: metadata.len(),
        modified: format_modified(metadata),
        dimensions,
    }
}

enum ScanSkip {
    Unreadable,
    Metadata,
}

fn scan_file(path: PathBuf, cancel_requested: &AtomicBool) -> Result<FileSnapshot, ScanSkip> {
    let parent = path.parent().expect("scanned file has a parent");
    let parent_identity = directory_identity(parent).map_err(|_| ScanSkip::Metadata)?;
    let mut file = open_canonical_file(&path).map_err(|_| ScanSkip::Unreadable)?;
    let stamp = FileStamp::read(&file).map_err(|_| ScanSkip::Metadata)?;
    let hash = compute_md5(&mut file, Some(cancel_requested)).map_err(|_| ScanSkip::Unreadable)?;
    if FileStamp::read(&file).map_err(|_| ScanSkip::Metadata)? != stamp {
        return Err(ScanSkip::Metadata);
    }
    Ok(FileSnapshot {
        path,
        parent_identity,
        stamp,
        hash,
    })
}

fn scan_folder_files(
    folder: &Path,
    scan_number: u64,
    cancel_requested: &AtomicBool,
    progress: impl Fn(ScanProgress) + Sync,
) -> Result<(ScanResult, ScanFiles), String> {
    let report = |message: String, fraction| progress(ScanProgress { message, fraction });
    report("Scanning for image files...".into(), 0.0);
    let root_error = |error| format!("Cannot scan folder {}: {}", folder.display(), error);
    let root = fs::canonicalize(folder).map_err(root_error)?;
    fs::read_dir(&root).map_err(root_error)?;

    let mut skipped = Skipped::default();
    let mut directories = HashSet::new();
    let mut paths = HashSet::new();
    let mut entries = WalkDir::new(root).follow_links(true).into_iter();
    while let Some(entry) = entries.next() {
        check_cancelled(cancel_requested)?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.depth() == 0 => {
                return Err(format!(
                    "Cannot scan folder {}: {}",
                    folder.display(),
                    error
                ));
            }
            Err(_) => {
                skipped.entries += 1;
                continue;
            }
        };

        if entry.file_type().is_dir() {
            match directory_identity(entry.path()) {
                Ok(identity) if !directories.insert(identity) => entries.skip_current_dir(),
                Ok(_) => {}
                Err(error) if entry.depth() == 0 => return Err(root_error(error)),
                Err(_) => {
                    entries.skip_current_dir();
                    skipped.entries += 1;
                }
            }
        } else if entry.file_type().is_file()
            && !entry.path_is_symlink()
            && is_image_file(entry.path())
        {
            let parent = entry.path().parent().expect("walked file has a parent");
            match fs::canonicalize(parent) {
                Ok(parent) => {
                    let path = parent.join(entry.file_name());
                    #[cfg(windows)]
                    let path = dunce::simplified(&path).to_path_buf();
                    paths.insert(path);
                }
                Err(_) => skipped.entries += 1,
            }
        }
    }

    let total = paths.len();
    report(
        format!("Found {} image files. Computing hashes...", total),
        0.05,
    );
    let completed = AtomicUsize::new(0);
    let hashes = paths
        .into_par_iter()
        .map(|path| {
            check_cancelled(cancel_requested)?;
            let result = scan_file(path, cancel_requested);
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if done % 50 == 0 || done == total {
                let fraction = 0.05 + 0.85 * (done as f64 / total as f64);
                report(format!("Hashing... {}/{}", done, total), fraction);
            }
            Ok(result)
        })
        .collect::<Result<Vec<_>, String>>()?;
    check_cancelled(cancel_requested)?;

    report("Grouping duplicates...".into(), 0.92);
    let mut by_hash = HashMap::<_, Vec<_>>::new();
    for result in hashes {
        match result {
            Ok(snapshot) => by_hash.entry(snapshot.hash).or_default().push(snapshot),
            Err(ScanSkip::Unreadable) => skipped.unreadable_files += 1,
            Err(ScanSkip::Metadata) => skipped.metadata_files += 1,
        }
    }

    report("Building file info...".into(), 0.95);
    let mut groups = Vec::new();
    let mut next_id = 0usize;
    for mut snapshots in by_hash.into_values().filter(|files| files.len() >= 2) {
        snapshots.sort_by(|a, b| a.path.cmp(&b.path));
        let mut group = Vec::new();
        for snapshot in snapshots {
            check_cancelled(cancel_requested)?;
            let metadata = match snapshot.open_checked().and_then(|file| file.metadata()) {
                Ok(metadata) => metadata,
                Err(_) => {
                    skipped.metadata_files += 1;
                    continue;
                }
            };
            let id = format!("{scan_number}:{next_id}");
            next_id += 1;
            let info = build_file_info(&snapshot.path, &metadata, Some(id.clone()));
            if snapshot.open_checked().is_err() {
                skipped.metadata_files += 1;
                continue;
            }
            group.push((id, info, snapshot));
        }
        if group.len() >= 2 {
            groups.push(group);
        }
    }
    groups.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| a[0].2.path.cmp(&b[0].2.path))
    });

    let mut duplicates = ScanFiles::default();
    let groups: Vec<_> = groups
        .into_iter()
        .enumerate()
        .map(|(group, rows)| {
            let mut ids = Vec::new();
            let mut files = Vec::new();
            for (id, info, snapshot) in rows {
                ids.push(id.clone());
                duplicates.files.insert(id, ScannedFile { snapshot, group });
                files.push(info);
            }
            duplicates.groups.push(ids);
            DuplicateGroup { files }
        })
        .collect();
    check_cancelled(cancel_requested)?;
    report(
        format!("Done. Found {} duplicate groups.", groups.len()),
        1.0,
    );
    Ok((ScanResult { groups, skipped }, duplicates))
}

fn matching_bytes(target: &mut File, survivor: &mut File) -> io::Result<bool> {
    use std::io::BufRead;

    target.rewind()?;
    let mut left = BufReader::new(target);
    let mut right = BufReader::new(survivor);
    loop {
        let left_bytes = left.fill_buf()?;
        let right_bytes = right.fill_buf()?;
        let len = left_bytes.len().min(right_bytes.len());
        if len == 0 {
            return Ok(left_bytes.is_empty() && right_bytes.is_empty());
        }
        if left_bytes[..len] != right_bytes[..len] {
            return Ok(false);
        }
        left.consume(len);
        right.consume(len);
    }
}

impl ScanFiles {
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        self.files
            .get(id)
            .map(|file| file.snapshot.path.clone())
            .ok_or_else(|| "The file is no longer in the active scan.".into())
    }

    fn remove_files(
        &mut self,
        ids: Vec<String>,
        keeper_id: Option<&str>,
        allow_remove_all: bool,
        remove: impl Fn(&Path) -> Result<(), String>,
    ) -> ActionResult {
        let mut selected = HashSet::new();
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| selected.insert(id.clone()))
            .collect();
        let group = ids
            .first()
            .and_then(|id| self.files.get(id))
            .map(|file| file.group);
        let valid_selection = group.is_some()
            && ids.iter().all(|id| {
                self.files
                    .get(id)
                    .is_some_and(|file| Some(file.group) == group)
            });
        let mut result = ActionResult {
            succeeded_ids: Vec::new(),
            failures: Vec::new(),
        };
        if !valid_selection {
            result
                .failures
                .extend(ids.into_iter().map(|id| ActionFailure {
                    id,
                    error: "Select files from one group in the active scan.".into(),
                }));
            return result;
        }

        let group = group.expect("validated group");
        let unselected: Vec<_> = self.groups[group]
            .iter()
            .filter(|id| !selected.contains(*id))
            .filter_map(|id| self.files.get(id).map(|file| (id.as_str(), &file.snapshot)))
            .collect();
        let remove_all = allow_remove_all && keeper_id.is_none() && unselected.is_empty();
        let survivors: Vec<_> = unselected
            .into_iter()
            .filter(|(id, _)| keeper_id.is_none_or(|keeper| keeper == *id))
            .map(|(_, snapshot)| snapshot)
            .collect();

        for id in ids {
            let snapshot = &self.files[&id].snapshot;
            let removal = (|| {
                let mut file = snapshot.open_verified().map_err(|e| e.to_string())?;
                if !remove_all
                    && !survivors.iter().any(|survivor| {
                        !survivor.same_entry(snapshot)
                            && survivor.stamp.size == snapshot.stamp.size
                            && survivor.open_checked().is_ok_and(|mut other| {
                                matching_bytes(&mut file, &mut other).unwrap_or(false)
                                    && survivor.open_checked().is_ok()
                            })
                    })
                {
                    return Err(
                        "No unchanged, matching file to keep. Scan again before removing files."
                            .into(),
                    );
                }
                // shortcut: path checks and removal are separate OS operations, use handle-relative
                // removal where available if concurrent filesystem changes must be fully excluded.
                snapshot.open_checked().map_err(|e| e.to_string())?;
                remove(&snapshot.path)
            })();
            match removal {
                Ok(()) => result.succeeded_ids.push(id),
                Err(error) => result.failures.push(ActionFailure { id, error }),
            }
        }
        for id in &result.succeeded_ids {
            self.files.remove(id);
        }
        self.groups[group].retain(|id| self.files.contains_key(id));
        if self.groups[group].len() < 2 {
            if let Some(id) = self.groups[group].pop() {
                self.files.remove(&id);
            }
        }
        result
    }
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
    let (scan_number, cancel_requested) = {
        let mut state = state.lock().map_err(|e| format!("Lock error: {}", e))?;
        if state.cancel_requested.is_some() {
            return Err("A scan is already running.".into());
        }
        state.scan_number += 1;
        state.duplicates = ScanFiles::default();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        state.cancel_requested = Some(Arc::clone(&cancel_requested));
        (state.scan_number, cancel_requested)
    };

    let scan_cancel = Arc::clone(&cancel_requested);
    let result = tauri::async_runtime::spawn_blocking(move || {
        scan_folder_files(Path::new(&folder), scan_number, &scan_cancel, |progress| {
            let _ = app.emit("scan-progress", progress);
        })
    })
    .await;

    let mut state = state.lock().map_err(|e| format!("Lock error: {}", e))?;
    state.cancel_requested = None;
    check_cancelled(&cancel_requested)?;
    let (result, duplicates) = result.map_err(|e| format!("Task join error: {}", e))??;
    state.duplicates = duplicates;
    Ok(result)
}

#[tauri::command(async)]
fn cancel_scan(state: tauri::State<'_, Mutex<AppState>>) -> Result<(), String> {
    let state = state.lock().map_err(|e| format!("Lock error: {}", e))?;
    if let Some(cancel_requested) = &state.cancel_requested {
        cancel_requested.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
fn get_file_info(path: String) -> Result<FileInfo, String> {
    let path = Path::new(&path);
    let metadata = fs::metadata(path).map_err(|e| format!("Metadata error: {}", e))?;
    Ok(build_file_info(path, &metadata, None))
}

#[tauri::command(async)]
fn get_thumbnail(id: String, state: tauri::State<'_, Mutex<AppState>>) -> Result<String, String> {
    let path = state
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .duplicates
        .path(&id)?;
    let img =
        image::open(&path).map_err(|e| format!("Cannot open image {}: {}", path.display(), e))?;
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
async fn trash_files(
    ids: Vec<String>,
    keeper_id: Option<String>,
    allow_remove_all: bool,
    app: AppHandle,
) -> Result<ActionResult, String> {
    remove_files(app, ids, keeper_id, allow_remove_all, |path| {
        // shortcut: the macOS Trash library requires UTF-8 filenames, lift this guard once it accepts native paths.
        #[cfg(target_os = "macos")]
        if path.to_str().is_none() {
            return Err("Trash on macOS cannot safely move this filename.".into());
        }
        // shortcut: the Windows Trash library strips verbatim prefixes, lift this guard once it preserves them.
        #[cfg(windows)]
        if matches!(
            path.components().next(),
            Some(std::path::Component::Prefix(prefix)) if prefix.kind().is_verbatim()
        ) {
            return Err("Trash on Windows cannot safely move this path.".into());
        }
        trash::delete(path).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn delete_files(
    ids: Vec<String>,
    keeper_id: Option<String>,
    allow_remove_all: bool,
    app: AppHandle,
) -> Result<ActionResult, String> {
    remove_files(app, ids, keeper_id, allow_remove_all, |path| {
        fs::remove_file(path).map_err(|e| e.to_string())
    })
    .await
}

async fn remove_files(
    app: AppHandle,
    ids: Vec<String>,
    keeper_id: Option<String>,
    allow_remove_all: bool,
    remove: impl Fn(&Path) -> Result<(), String> + Send + 'static,
) -> Result<ActionResult, String> {
    use tauri::Manager;

    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Mutex<AppState>>();
        let mut state = state.lock().map_err(|e| format!("Lock error: {}", e))?;
        Ok(state
            .duplicates
            .remove_files(ids, keeper_id.as_deref(), allow_remove_all, remove))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

#[tauri::command(async)]
fn open_file(id: String, state: tauri::State<'_, Mutex<AppState>>) -> Result<(), String> {
    let path = state
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .duplicates
        .path(&id)?;
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(path)
        .spawn()
        .map_err(|e| format!("Failed to open file: {}", e))?;
    Ok(())
}

#[tauri::command(async)]
fn reveal_file(id: String, state: tauri::State<'_, Mutex<AppState>>) -> Result<(), String> {
    let path = state
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .duplicates
        .path(&id)?;
    #[cfg(target_os = "macos")]
    let command = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn();
    #[cfg(target_os = "windows")]
    let command = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(&path)
        .spawn();
    #[cfg(target_os = "linux")]
    let command = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(&path))
        .spawn();
    command.map_err(|e| format!("Failed to reveal file: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn keep_first_preserves_original_when_a_folder_is_linked() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let photos = directory.path().join("photos");
        let alias = directory.path().join("alias");
        fs::create_dir(&photos).unwrap();
        symlink(&photos, &alias).unwrap();
        let original = photos.join("first.jpg");
        let duplicate = photos.join("second.jpg");
        let bytes = b"matching image bytes";
        fs::write(&original, bytes).unwrap();
        fs::write(&duplicate, bytes).unwrap();

        let (result, mut scan) =
            scan_folder_files(directory.path(), 1, &AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!(result.groups.len(), 1);
        let files = &result.groups[0].files;
        assert_eq!(files.len(), 2);
        let keeper = files[0].id.as_deref().unwrap();
        assert_eq!(
            scan.path(keeper).unwrap(),
            dunce::canonicalize(&original).unwrap()
        );
        let selected = files[1].id.clone().unwrap();
        assert_eq!(
            scan.path(&selected).unwrap(),
            dunce::canonicalize(&duplicate).unwrap()
        );

        let outcome = scan.remove_files(vec![selected.clone()], Some(keeper), false, |path| {
            fs::remove_file(path).map_err(|error| error.to_string())
        });

        assert!(outcome.failures.is_empty());
        assert_eq!(outcome.succeeded_ids, vec![selected]);
        assert_eq!(fs::read(original).unwrap(), bytes);
        assert_eq!(fs::read(alias.join("first.jpg")).unwrap(), bytes);
        assert!(!duplicate.exists());
    }

    #[test]
    fn partial_removal_keeps_the_failed_file_and_its_keeper() {
        let root = tempfile::tempdir().unwrap();
        let names = ["01-keeper.png", "02-failed.png", "03-removed.png"];
        for name in names {
            fs::write(root.path().join(name), b"matching image bytes").unwrap();
        }
        let (result, mut scan) =
            scan_folder_files(root.path(), 1, &AtomicBool::new(false), |_| {}).unwrap();
        let files = &result.groups[0].files;
        let keeper = files[0].id.as_ref().unwrap();
        let failed = files[1].id.as_ref().unwrap();
        let removed = files[2].id.as_ref().unwrap();

        let outcome = scan.remove_files(
            vec![failed.clone(), removed.clone()],
            Some(keeper),
            false,
            |path| {
                if path.ends_with(names[1]) {
                    return Err("Permission denied".into());
                }
                fs::remove_file(path).map_err(|error| error.to_string())
            },
        );

        assert_eq!(outcome.succeeded_ids, [removed.clone()]);
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(&outcome.failures[0].id, failed);
        assert_eq!(scan.groups, [vec![keeper.clone(), failed.clone()]]);
        assert!(scan.path(removed).is_err());
        assert!(root.path().join(names[0]).is_file());
        assert!(root.path().join(names[1]).is_file());
        assert!(!root.path().join(names[2]).exists());
    }
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

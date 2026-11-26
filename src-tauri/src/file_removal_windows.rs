use super::{windows_file_identity, FileIdentity, REMOVAL_STAGE_PREFIX, REMOVAL_STAGE_RANDOM_LEN};
use std::fs::{self, File};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::ptr;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, MoveFileExW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_TRAVERSE,
};

fn lock_directory(path: &Path) -> io::Result<File> {
    let file = File::options()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::other("The directory changed since the scan."));
    }
    Ok(file)
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    // Protect the stage from inherited permissions in shared directories.
    let sddl: Vec<_> = "D:P(A;;FA;;;OW)(A;;FA;;;SY)\0".encode_utf16().collect();
    let path = wide_path(path);
    let mut descriptor = ptr::null_mut();
    // The Win32 call allocates the security descriptor, which is freed below.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = if CreateDirectoryW(path.as_ptr(), &attributes) != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
        LocalFree(descriptor);
        result
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn move_without_replacing(from: &Path, to: &Path) -> io::Result<()> {
    let from = wide_path(from);
    let to = wide_path(to);
    // Zero flags require a same-volume move and prohibit replacing the destination.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(super) struct StagedFile<'a> {
    pub(super) path: &'a Path,
}

impl StagedFile<'_> {
    pub(super) fn delete(&self) -> Result<(), String> {
        fs::remove_file(self.path).map_err(|error| error.to_string())
    }

    pub(super) fn trash(&self) -> Result<(), String> {
        if matches!(
            self.path.components().next(),
            Some(std::path::Component::Prefix(prefix)) if prefix.kind().is_verbatim()
        ) {
            return Err("Trash on Windows cannot safely move this path.".into());
        }
        trash::delete(self.path).map_err(|error| error.to_string())
    }
}

pub(super) fn with_staged_file(
    path: &Path,
    expected_parent: FileIdentity,
    action: impl FnOnce(&StagedFile<'_>, File) -> Result<(), String>,
) -> Result<(), String> {
    let parent_path = path.parent().expect("a scanned file has a parent");
    let name = path.file_name().expect("a scanned file has a name");
    let mut ancestors: Vec<_> = parent_path
        .ancestors()
        .take_while(|dir| dir.is_absolute())
        .collect();
    ancestors.reverse();
    // Hold the directory chain from root to leaf while using path-based Win32 APIs.
    let directories = ancestors
        .into_iter()
        .map(lock_directory)
        .collect::<io::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let parent = directories.last().expect("the scanned path is absolute");
    if windows_file_identity(parent).map_err(|error| error.to_string())? != expected_parent {
        return Err("The directory changed since the scan.".into());
    }

    // Windows canonicalization provides a verbatim path for the longer staging name.
    let parent_path = fs::canonicalize(parent_path).map_err(|error| error.to_string())?;
    let original = parent_path.join(name);
    let stage = tempfile::Builder::new()
        .prefix(REMOVAL_STAGE_PREFIX)
        .rand_bytes(REMOVAL_STAGE_RANDOM_LEN)
        .disable_cleanup(true)
        .make_in(parent_path, create_private_directory)
        .map_err(|error| error.to_string())?;
    let stage_path = stage.path();
    let stage_guard = lock_directory(stage_path).map_err(|error| error.to_string())?;
    let staged = stage_path.join(name);
    if let Err(error) = move_without_replacing(&original, &staged) {
        drop(stage_guard);
        let _ = fs::remove_dir(stage_path);
        return Err(error.to_string());
    }

    let result = File::options()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&staged)
        .and_then(|file| {
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(io::Error::other("The file changed since the scan."));
            }
            Ok(file)
        })
        .map_err(|error| error.to_string())
        .and_then(|file| {
            action(
                &StagedFile {
                    path: dunce::simplified(&staged),
                },
                file,
            )
        });

    if let Err(error) = &result {
        if let Err(restore_error) = move_without_replacing(&staged, &original) {
            return Err(format!(
                "{error} Could not restore {}: {restore_error}. Check {} for the staged file.",
                path.display(),
                dunce::simplified(&staged).display(),
            ));
        }
    }
    drop(stage_guard);
    let _ = fs::remove_dir(stage_path);
    result
}

use super::{windows_file_identity, FileIdentity, REMOVAL_STAGE_PREFIX, REMOVAL_STAGE_RANDOM_LEN};
use std::fs::{self, File};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr;
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_CREATE, FILE_DIRECTORY_FILE, FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Foundation::{
    LocalFree, RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, UNICODE_STRING,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetVolumeInformationByHandleW, MoveFileExW, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, SYNCHRONIZE,
};
use windows_sys::Win32::System::SystemServices::FILE_PERSISTENT_ACLS;
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

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

fn require_persistent_acls(parent: &File) -> io::Result<()> {
    let mut flags = 0;
    // Only the flags output is requested for this live directory handle.
    if unsafe {
        GetVolumeInformationByHandleW(
            parent.as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if flags & FILE_PERSISTENT_ACLS == 0 {
        return Err(io::Error::other(
            "This volume does not enforce the permissions required for safe removal.",
        ));
    }
    Ok(())
}

fn create_private_directory(parent: &File, candidate: &Path) -> io::Result<File> {
    let mut name: Vec<_> = candidate
        .file_name()
        .expect("generated stage name")
        .encode_wide()
        .collect();
    let length = u16::try_from(std::mem::size_of_val(name.as_slice()))
        .map_err(|_| io::Error::other("The recovery folder name is too long."))?;
    let name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: name.as_mut_ptr(),
    };
    let sddl: Vec<_> = "D:P(A;;FA;;;OW)(A;;FA;;;SY)\0".encode_utf16().collect();
    let mut descriptor = ptr::null_mut();
    // Supply the protected ACL and acquire the directory handle in one operation.
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
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle(),
            ObjectName: &name,
            Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
            SecurityDescriptor: descriptor.cast(),
            SecurityQualityOfService: ptr::null(),
        };
        let mut stage_handle = ptr::null_mut();
        let status = NtCreateFile(
            &mut stage_handle,
            FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE,
            &attributes,
            &mut IO_STATUS_BLOCK::default(),
            ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_CREATE,
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(),
            0,
        );
        LocalFree(descriptor);
        if status < 0 {
            return Err(io::Error::from_raw_os_error(
                RtlNtStatusToDosError(status) as i32
            ));
        }
        Ok(File::from_raw_handle(stage_handle))
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
        // shortcut: the Windows Trash library strips verbatim prefixes, lift this guard when it preserves them.
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
    require_persistent_acls(parent).map_err(|error| error.to_string())?;

    // Windows canonicalization provides a verbatim path for the longer staging name.
    let parent_path = fs::canonicalize(parent_path).map_err(|error| error.to_string())?;
    let original = parent_path.join(name);
    let stage = tempfile::Builder::new()
        .prefix(REMOVAL_STAGE_PREFIX)
        .rand_bytes(REMOVAL_STAGE_RANDOM_LEN)
        .disable_cleanup(true)
        .make_in(parent_path, |candidate| {
            create_private_directory(parent, candidate)
        })
        .map_err(|error| error.to_string())?;
    let (stage_guard, stage_path) = stage.into_parts();
    let staged = stage_path.join(name);
    if let Err(error) = move_without_replacing(&original, &staged) {
        drop(stage_guard);
        let _ = fs::remove_dir(&stage_path);
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
    let _ = fs::remove_dir(&stage_path);
    result
}

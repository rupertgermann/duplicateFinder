use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use rustix::fs::{self as unix, AtFlags, Mode, OFlags, RenameFlags};

use super::{unix_file_identity, FileIdentity, REMOVAL_STAGE_PREFIX, REMOVAL_STAGE_RANDOM_LEN};

pub(super) struct StagedFile<'a> {
    pub(super) path: &'a Path,
    directory: &'a File,
    #[cfg(target_os = "linux")]
    original_path: &'a Path,
}

impl StagedFile<'_> {
    pub(super) fn delete(&self) -> Result<(), String> {
        unix::unlinkat(self.directory, self.name(), AtFlags::empty())
            .map_err(|error| error.to_string())
    }

    fn name(&self) -> &std::ffi::OsStr {
        self.path.file_name().expect("staged file has a name")
    }

    #[cfg(target_os = "linux")]
    pub(super) fn trash(&self) -> Result<(), String> {
        super::file_removal_linux::trash(self.directory, self.name(), self.original_path)
    }

    #[cfg(target_os = "macos")]
    pub(super) fn trash(&self) -> Result<(), String> {
        use objc2_foundation::{
            NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask, NSString, NSURL,
        };

        // shortcut: the Trash URL bridge accepts UTF-8 paths, use native filesystem URLs to lift this guard.
        let text = self
            .path
            .to_str()
            .ok_or("Trash on macOS cannot safely move this filename.")?;
        let manager = NSFileManager::defaultManager();
        let url = NSURL::fileURLWithPath(&NSString::from_str(text));
        let temporary = manager
            .URLForDirectory_inDomain_appropriateForURL_create_error(
                NSSearchPathDirectory::ItemReplacementDirectory,
                NSSearchPathDomainMask::UserDomainMask,
                Some(&url),
                true,
            )
            .map_err(|error| error.to_string())?;
        let temp_path = temporary
            .path()
            .ok_or("Could not locate a temporary folder on this volume.")?
            .to_string();
        let result = (|| {
            let temp = unix::open(temp_path.as_str(), DIR_FLAGS, Mode::empty())
                .map_err(|error| error.to_string())?;
            let entry = temporary
                .URLByAppendingPathComponent(&NSString::from_str(
                    self.name().to_str().expect("checked UTF-8 path"),
                ))
                .ok_or("Could not create a URL for the file.")?;
            self.prepare_trash_entry(&temp)
                .map_err(|error| format!("Cannot prepare the file for the macOS Trash: {error}"))?;
            if let Err(error) = manager.trashItemAtURL_resultingItemURL_error(&entry, None) {
                let _ = unix::unlinkat(&temp, self.name(), AtFlags::empty());
                return Err(error.to_string());
            }
            if !matches!(
                unix::statat(&temp, self.name(), AtFlags::SYMLINK_NOFOLLOW),
                Err(rustix::io::Errno::NOENT)
            ) {
                return Err("Could not confirm that macOS moved the file to Trash.".into());
            }
            self.delete()
        })();
        let _ = std::fs::remove_dir(temp_path);
        result
    }

    #[cfg(target_os = "macos")]
    fn prepare_trash_entry(&self, temp: &impl std::os::fd::AsFd) -> io::Result<()> {
        use rustix::io::Errno;

        let name = self.name();
        match unix::linkat(self.directory, name, temp, name, AtFlags::empty()) {
            Ok(()) => Ok(()),
            Err(Errno::XDEV | Errno::NOTSUP | Errno::OPNOTSUPP | Errno::PERM) => {
                self.copy_trash_entry(temp)
            }
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(target_os = "macos")]
    fn copy_trash_entry(&self, temp: &impl std::os::fd::AsFd) -> io::Result<()> {
        let name = self.name();
        let from = File::from(unix::openat(
            self.directory,
            name,
            STAGED_FILE_FLAGS,
            Mode::empty(),
        )?);
        let stamp = super::FileStamp::read(&from)?;
        let to = File::from(unix::openat(
            temp,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        let copied = (|| {
            let state = unix::copyfile_state_alloc()?;
            // The state is uniquely owned until copyfile_state_free, even on failure.
            unsafe {
                let result = unix::fcopyfile(&from, &to, state, unix::CopyfileFlags::ALL);
                let freed = unix::copyfile_state_free(state);
                result.and(freed)?;
            }
            if super::FileStamp::read(&from)? != stamp || to.metadata()?.len() != stamp.size {
                return Err(io::Error::other(
                    "File changed while preparing it for Trash.",
                ));
            }
            to.sync_all()
        })();
        if copied.is_err() {
            let _ = unix::unlinkat(temp, name, AtFlags::empty());
        }
        copied
    }
}

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const STAGED_FILE_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);

pub(super) fn with_staged_file(
    path: &Path,
    expected_parent: FileIdentity,
    action: impl FnOnce(&StagedFile<'_>, File) -> Result<(), String>,
) -> Result<(), String> {
    let parent_path = path.parent().expect("scanned file has a parent");
    let name = path.file_name().expect("scanned file has a name");
    let parent = File::from(
        unix::open(parent_path, DIR_FLAGS, Mode::empty()).map_err(|error| error.to_string())?,
    );
    if unix_file_identity(&parent.metadata().map_err(|error| error.to_string())?) != expected_parent
    {
        return Err("The parent folder changed since the scan. Scan again.".into());
    }

    let stage = tempfile::Builder::new()
        .prefix(REMOVAL_STAGE_PREFIX)
        .rand_bytes(REMOVAL_STAGE_RANDOM_LEN)
        .disable_cleanup(true)
        .make_in(parent_path, |candidate| {
            let name = candidate.file_name().expect("generated stage name");
            unix::mkdirat(&parent, name, Mode::RWXU).map_err(io::Error::from)
        })
        .map_err(|error| format!("Cannot create a recovery folder: {error}"))?;
    let stage_name = stage.path().file_name().expect("generated stage name");

    let result = (|| {
        let stage_dir = File::from(
            unix::openat(&parent, stage_name, DIR_FLAGS, Mode::empty())
                .map_err(|error| error.to_string())?,
        );
        let metadata = stage_dir.metadata().map_err(|error| error.to_string())?;
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err("The recovery folder is not private. The file was left in place.".into());
        }
        #[cfg(target_os = "macos")]
        clear_inherited_acl(&stage_dir).map_err(|error| error.to_string())?;
        unix::renameat_with(&parent, name, &stage_dir, name, RenameFlags::NOREPLACE)
            .map_err(|error| format!("Cannot secure the file for removal: {error}"))?;

        let staged_result = (|| {
            let file = File::from(
                unix::openat(&stage_dir, name, STAGED_FILE_FLAGS, Mode::empty())
                    .map_err(|error| error.to_string())?,
            );
            let stage_path = directory_path(&stage_dir).map_err(|error| error.to_string())?;
            action(
                &StagedFile {
                    path: &stage_path.join(name),
                    directory: &stage_dir,
                    #[cfg(target_os = "linux")]
                    original_path: path,
                },
                file,
            )
        })();

        if let Err(error) = staged_result {
            if let Err(restore_error) =
                unix::renameat_with(&stage_dir, name, &parent, name, RenameFlags::NOREPLACE)
            {
                let recovery = directory_path(&stage_dir)
                    .unwrap_or_else(|_| stage.path().to_path_buf())
                    .join(name);
                return Err(format!(
                    "{error} Could not restore {} to its original path: {restore_error}",
                    recovery.display()
                ));
            }
            return Err(error);
        }
        Ok(())
    })();

    // A failed restore or interrupted operation must never trigger recursive cleanup.
    let _ = unix::unlinkat(&parent, stage_name, AtFlags::REMOVEDIR);
    result
}

#[cfg(target_os = "macos")]
fn clear_inherited_acl(dir: &File) -> io::Result<()> {
    use std::ffi::{c_int, c_void};
    use std::os::fd::AsRawFd;

    extern "C" {
        fn acl_init(count: c_int) -> *mut c_void;
        fn acl_set_fd_np(fd: c_int, acl: *mut c_void, acl_type: c_int) -> c_int;
        fn acl_free(acl: *mut c_void) -> c_int;
    }

    const ACL_TYPE_EXTENDED: c_int = 0x00000100;
    // macOS ACLs can grant access despite mode 0700. Clear inherited entries on our new directory.
    unsafe {
        let acl = acl_init(0);
        if acl.is_null() {
            return Err(io::Error::last_os_error());
        }
        let result = acl_set_fd_np(dir.as_raw_fd(), acl, ACL_TYPE_EXTENDED);
        let error = io::Error::last_os_error();
        acl_free(acl);
        if result == 0 || error.kind() == io::ErrorKind::Unsupported {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(target_os = "macos")]
fn directory_path(dir: &File) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let path = unix::getpath(dir)?;
    Ok(PathBuf::from(OsString::from_vec(path.into_bytes())))
}

#[cfg(target_os = "linux")]
fn directory_path(dir: &File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;

    std::fs::read_link(format!("/proc/self/fd/{}", dir.as_raw_fd()))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn trash_copy_keeps_the_source_and_preserves_data_and_mode() {
        use std::os::unix::fs::PermissionsExt;

        let source = tempfile::tempdir().unwrap();
        let handoff = tempfile::tempdir().unwrap();
        let path = source.path().join("copy.png");
        fs::write(&path, b"image bytes").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let directory = File::open(source.path()).unwrap();
        let staged = StagedFile {
            path: &path,
            directory: &directory,
        };

        staged
            .copy_trash_entry(&File::open(handoff.path()).unwrap())
            .unwrap();

        let copy = handoff.path().join("copy.png");
        assert_eq!(fs::read(&path).unwrap(), b"image bytes");
        assert_eq!(fs::read(&copy).unwrap(), b"image bytes");
        assert_eq!(fs::metadata(copy).unwrap().mode() & 0o777, 0o640);
    }
}

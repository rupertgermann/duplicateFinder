use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, FileTimes};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use percent_encoding::{percent_encode, AsciiSet, NON_ALPHANUMERIC};
use rustix::fs::{self as unix, AtFlags, Mode, OFlags, RenameFlags, XattrFlags};
use rustix::io::Errno;

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const NEW_FILE_FLAGS: OFlags = OFlags::WRONLY
    .union(OFlags::CREATE)
    .union(OFlags::EXCL)
    .union(OFlags::CLOEXEC);
const PATH_ESCAPE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'/');

pub(super) fn trash(directory: &File, name: &OsStr, original: &Path) -> Result<(), String> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".local/share"))
        })
        .ok_or("Could not locate the Linux home Trash.")?;
    move_to_home_trash(&data_home, directory, name, original)
        .map_err(|error| format!("Could not move the file to the Linux home Trash: {error}"))
}

fn move_to_home_trash(
    data_home: &Path,
    directory: &File,
    name: &OsStr,
    original: &Path,
) -> io::Result<()> {
    fs::create_dir_all(data_home)?;
    let home = File::open(data_home)?;
    require_private_directory(&home)?;
    let trash = open_or_create_directory(&home, "Trash")?;
    let files = open_or_create_directory(&trash, "files")?;
    let info = open_or_create_directory(&trash, "info")?;

    // The freedesktop home Trash supports absolute original paths, including other volumes.
    let mut reservation = reserve_trash_entry(&info, name)?;
    let metadata_name = reservation
        .path()
        .file_name()
        .expect("generated metadata name")
        .to_os_string();
    let trash_name = Path::new(&metadata_name)
        .file_stem()
        .expect("generated file name");
    let original = percent_encode(original.as_os_str().as_bytes(), PATH_ESCAPE);
    let timezone =
        jiff::tz::TimeZone::try_system().unwrap_or_else(|_| jiff::tz::TimeZone::unknown());
    let date = jiff::Timestamp::now().to_zoned_datetime(timezone);
    let result = (|| -> io::Result<bool> {
        writeln!(
            reservation.as_file_mut(),
            "[Trash Info]\nPath={original}\nDeletionDate={}",
            date.strftime("%Y-%m-%dT%H:%M:%S")
        )?;
        reservation.as_file().sync_all()?;
        info.sync_all()?;
        if !same_entry(&info, &metadata_name, reservation.as_file()) {
            return Err(io::Error::other("The Linux Trash metadata entry changed."));
        }
        match unix::renameat_with(directory, name, &files, trash_name, RenameFlags::NOREPLACE) {
            Ok(()) => Ok(false),
            Err(Errno::XDEV) => {
                copy_to_trash(directory, name, &trash, &files, trash_name)?;
                Ok(true)
            }
            Err(error) => Err(error.into()),
        }
    })();
    match result {
        Ok(false) => Ok(()),
        Ok(true) => {
            // The Trash hierarchy may be new on a different volume than the staged original.
            unix::syncfs(&files)?;
            unix::unlinkat(directory, name, AtFlags::empty())?;
            Ok(())
        }
        Err(error) => {
            if same_entry(&info, &metadata_name, reservation.as_file()) {
                let _ = unix::unlinkat(&info, &metadata_name, AtFlags::empty());
            }
            Err(error)
        }
    }
}

fn open_or_create_directory(parent: &File, name: &str) -> io::Result<File> {
    match unix::mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => (),
        Err(error) => return Err(error.into()),
    }
    let dir = File::from(unix::openat(parent, name, DIR_FLAGS, Mode::empty())?);
    require_private_directory(&dir)?;
    Ok(dir)
}

fn require_private_directory(dir: &File) -> io::Result<()> {
    let metadata = dir.metadata()?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err(io::Error::other("The Linux Trash directory is writable by another account or is not owned by this user."));
    }
    Ok(())
}

fn reserve_trash_entry(dir: &File, name: &OsStr) -> io::Result<tempfile::NamedTempFile<File>> {
    let name = Path::new(name);
    let mut prefix = OsString::from("duplicate-finder");
    let mut suffix = OsString::new();
    if name.as_os_str().len() <= 200 {
        prefix = name.file_stem().expect("staged file has a name").into();
        if let Some(extension) = name.extension() {
            suffix.push(".");
            suffix.push(extension);
        }
    }
    prefix.push("-");
    suffix.push(".trashinfo");
    new_temporary_entry(dir, &prefix, &suffix)
}

fn new_temporary_entry(
    dir: &File,
    prefix: &OsStr,
    suffix: &OsStr,
) -> io::Result<tempfile::NamedTempFile<File>> {
    tempfile::Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .rand_bytes(20)
        .disable_cleanup(true)
        .make_in(".", |path| {
            let name = path.file_name().expect("generated Trash filename");
            let fd = unix::openat(dir, name, NEW_FILE_FLAGS, Mode::RUSR | Mode::WUSR)?;
            Ok(File::from(fd))
        })
}

fn same_entry(dir: &File, name: &OsStr, opened: &File) -> bool {
    let Ok(metadata) = opened.metadata() else {
        return false;
    };
    unix::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW)
        .is_ok_and(|entry| (entry.st_dev, entry.st_ino) == (metadata.dev(), metadata.ino()))
}

fn copy_to_trash(
    directory: &File,
    name: &OsStr,
    trash: &File,
    files: &File,
    trash_name: &OsStr,
) -> io::Result<()> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut source = File::from(unix::openat(directory, name, flags, Mode::empty())?);
    let before = source.metadata()?;
    if !before.is_file() {
        return Err(io::Error::other(
            "The staged file is no longer a regular file.",
        ));
    }
    let mut copy =
        new_temporary_entry(trash, OsStr::new(".duplicate-finder-copy-"), OsStr::new(""))?;
    let result = (|| -> io::Result<()> {
        let written = io::copy(&mut source, copy.as_file_mut())?;
        let after = source.metadata()?;
        if written != before.len() || !same_contents(&before, &after) {
            return Err(io::Error::other(
                "The staged file changed during the Trash transfer.",
            ));
        }
        copy.as_file().set_permissions(before.permissions())?;
        copy_user_metadata(&source, copy.as_file())?;
        copy.as_file().set_times(
            FileTimes::new()
                .set_accessed(before.accessed()?)
                .set_modified(before.modified()?),
        )?;
        copy.as_file().sync_all()?;
        if !same_contents(&before, &source.metadata()?) {
            return Err(io::Error::other(
                "The staged file changed during the Trash transfer.",
            ));
        }
        unix::renameat_with(
            trash,
            copy.path().file_name().expect("generated copy name"),
            files,
            trash_name,
            RenameFlags::NOREPLACE,
        )?;
        Ok(())
    })();
    if result.is_err() {
        let _ = unix::unlinkat(
            trash,
            copy.path().file_name().expect("generated copy name"),
            AtFlags::empty(),
        );
    }
    result
}

fn same_contents(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    (
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    ) == (
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
        after.ctime(),
        after.ctime_nsec(),
    )
}

fn copy_user_metadata(source: &File, copy: &File) -> io::Result<()> {
    let length = match unix::flistxattr_query(source) {
        Ok(length) => length,
        Err(Errno::OPNOTSUPP) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if length == 0 {
        return Ok(());
    }
    let mut names = vec![0; length];
    let used = unix::flistxattr(source, names.as_mut_slice())?;
    for key in names[..used]
        .split(|byte| *byte == 0)
        .filter(|key| key.starts_with(b"user.") || *key == b"system.posix_acl_access")
    {
        let key = OsStr::from_bytes(key);
        let mut value = vec![0; unix::fgetxattr_query(source, key)?];
        let used = if value.is_empty() {
            0
        } else {
            unix::fgetxattr(source, key, value.as_mut_slice())?
        };
        unix::fsetxattr(copy, key, &value[..used], XattrFlags::empty())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_trash_consumes_pinned_entry_and_preserves_original_restore_path() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        let moved = root.path().join("moved");
        let data_home = root.path().join("data");
        let name = OsStr::new("photo.jpg");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join(name), b"selected").unwrap();
        let directory = File::open(&stage).unwrap();
        fs::rename(&stage, &moved).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join(name), b"replacement").unwrap();
        let original = root
            .path()
            .join(OsStr::from_bytes(b"photos/% pic\n\xff.jpg"));

        move_to_home_trash(&data_home, &directory, name, &original).unwrap();

        assert!(!moved.join(name).exists());
        assert_eq!(fs::read(stage.join(name)).unwrap(), b"replacement");
        let stored = fs::read_dir(data_home.join("Trash/files"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(stored.path()).unwrap(), b"selected");
        let mut metadata_name = stored.file_name();
        metadata_name.push(".trashinfo");
        let metadata =
            fs::read_to_string(data_home.join("Trash/info").join(metadata_name)).unwrap();
        let path = metadata
            .lines()
            .find_map(|line| line.strip_prefix("Path="))
            .unwrap();
        let restored: Vec<u8> = percent_encoding::percent_decode_str(path).collect();
        assert_eq!(restored, original.as_os_str().as_bytes());
        assert!(metadata
            .lines()
            .any(|line| line.starts_with("DeletionDate=")));
    }
}

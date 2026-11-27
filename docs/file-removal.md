# File removal and recovery

**Move to Trash** and **Delete Permanently** act on the checked files in one duplicate group. Both actions require confirmation. If you check every file manually, the confirmation warns that the action removes the entire group.

## Checks before removal

The app checks that the selected path refers to the same file and parent directory as the scan. It then moves the selected file into a private `.duplicate-finder-recovery-*` folder inside its original parent and validates the staged file against the scan.

Unless you confirm removal of the entire group, each selected file needs a separate, unchanged, byte-for-byte match in the group. **Select All Except First** uses the first file in path order as the copy to keep. For a manual selection, the app can check any unselected copy.

If an action fails after staging, the app tries to return the file to its original name without replacing a file that occupies that path. Hover over the status message to read the full details of the first failure in the batch.

## Recover a staged file

1. Check the status bar for a recovery path. After an interruption, look in the original folder for a `.duplicate-finder-recovery-*` directory. On macOS or Linux, show hidden files to see it.
2. Check whether the original name is occupied. If it is, choose a different name or a different destination before moving the staged file.
3. Move the staged file out of the recovery folder. Scan the original folder again to review its current contents.

## Trash behavior by platform

| Platform | Restore location and requirements |
| --- | --- |
| macOS | Move the item out of Trash to the original folder manually. **Move to Trash** reports an error for paths containing invalid UTF-8. |
| Windows | Move the item out of the Recycle Bin to the original folder manually. Removal requires a file system with persistent access controls. **Move to Trash** reports an error for paths that require a verbatim prefix, including UNC network paths. |
| Linux | The app records the original path in the user's home Trash, including for files from other volumes. The home Trash is under `$XDG_DATA_HOME/Trash` when `XDG_DATA_HOME` is an absolute path. Otherwise, it uses `~/.local/share/Trash`. For cross-volume moves, the app copies the file and syncs the Trash volume before removing the staged copy. |

**Delete Permanently** removes the staged file directly. Use **Move to Trash** when you want to be able to restore it later.

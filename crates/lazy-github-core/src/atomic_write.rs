//! Writing a whole file so that a failure part-way (a full disk, a read-only folder, a crash) leaves the previous
//! file as it was: the text goes to a temp file next to the target, which is then renamed over it.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// Replaces the file at `path` with `contents`, which `write_all` puts into the temp file (`Write::write_all`,
/// except in tests that fill the disk part-way). The folder must exist.
///
/// A symlink at `path` is followed, so the link stays and its target is replaced, as an in-place write would do. An
/// existing file's permissions are kept.
pub(crate) fn write_with(
    path: &Path,
    contents: &[u8],
    write_all: impl FnOnce(&mut dyn Write, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    // `canonicalize` fails when the file doesn't exist yet, which is the first write.
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let mut tmp_name = target.file_name().unwrap_or_default().to_owned();
    tmp_name.push(".tmp");
    let tmp = target.with_file_name(tmp_name);

    let result = (|| {
        let mut file = File::create(&tmp)?;
        write_all(&mut file, contents)?;
        // Keep the mode of an existing file (say, `chmod 600`), as an in-place write would.
        if let Ok(metadata) = std::fs::metadata(&target) {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        // Best effort: the original error is the one worth reporting, and a temp file that couldn't be created
        // isn't there to remove.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fs::disk_full_after;

    fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
        write_with(path, contents, |file, bytes| file.write_all(bytes))
    }

    #[test]
    fn replaces_the_file_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        write(&path, b"one").unwrap();
        write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["f.json"]);
    }

    #[test]
    fn a_full_disk_leaves_the_previous_file_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        write(&path, b"previous").unwrap();
        let error = write_with(&path, b"the new, longer text", disk_full_after(4)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert!(!dir.path().join("f.json.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_keeps_pointing_at_the_replaced_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.json");
        let link = dir.path().join("link.json");
        std::fs::write(&real, "old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
    }
}

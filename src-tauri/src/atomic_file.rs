//! Temporary files are exclusive and live alongside their destination.
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
use tempfile::{Builder, NamedTempFile};

pub fn temporary(dest: &Path) -> io::Result<NamedTempFile> {
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent"))?;
    fs::create_dir_all(parent)?;
    Builder::new()
        .prefix(".yuru-")
        .suffix(".part")
        .tempfile_in(parent)
}

pub fn commit(temp: NamedTempFile, dest: &Path) -> io::Result<()> {
    temp.as_file().sync_all()?;
    temp.persist(dest).map_err(|err| err.error)?;
    Ok(())
}

pub fn write(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temp = temporary(dest)?;
    temp.write_all(bytes)?;
    commit(temp, dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_is_atomic_and_preserves_foreign_part_files() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file");
        fs::write(dir.path().join("file.part"), "foreign").unwrap();
        write(&dest, b"old").unwrap();
        write(&dest, b"new").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert_eq!(fs::read(dir.path().join("file.part")).unwrap(), b"foreign");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

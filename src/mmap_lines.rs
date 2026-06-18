use anyhow::Result;
use std::path::Path;

pub fn mmap_file(path: &Path) -> Result<Option<memmap2::Mmap>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() == 0 {
        return Ok(None);
    }
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    Ok(Some(mmap))
}

pub fn iter_lines(mmap: &memmap2::Mmap) -> impl Iterator<Item = &[u8]> {
    mmap.split(|&b| b == b'\n')
        .map(|slice| {
            let end = slice
                .len()
                .saturating_sub(if slice.ends_with(b"\r") { 1 } else { 0 });
            &slice[..end]
        })
        .filter(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_mmap_file_nonexistent() {
        let path = std::path::PathBuf::from("/tmp/ob_test_nonexistent_12345");
        let _ = std::fs::remove_file(&path);
        let result = mmap_file(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_mmap_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::File::create(&path).unwrap();
        let result = mmap_file(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_iter_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello\nworld\nfoo\n").unwrap();
        drop(f);

        let mmap = mmap_file(&path).unwrap().unwrap();
        let lines: Vec<&[u8]> = iter_lines(&mmap).collect();
        assert_eq!(
            lines,
            vec![b"hello".as_slice(), b"world".as_slice(), b"foo".as_slice()]
        );
    }

    #[test]
    fn test_iter_lines_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trailing.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"a\nb\n").unwrap();
        drop(f);

        let mmap = mmap_file(&path).unwrap().unwrap();
        let lines: Vec<&[u8]> = iter_lines(&mmap).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], b"a");
        assert_eq!(lines[1], b"b");
    }

    #[test]
    fn test_iter_lines_blank_lines_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blanks.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"a\n\n\nb\n").unwrap();
        drop(f);

        let mmap = mmap_file(&path).unwrap().unwrap();
        let lines: Vec<&[u8]> = iter_lines(&mmap).collect();
        assert_eq!(lines, vec![b"a".as_slice(), b"b".as_slice()]);
    }
}

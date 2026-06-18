use anyhow::Result;
use std::io::Write;
use std::path::Path;

pub fn generate_forget_set(ob_dir: &Path, tokenizer: &str) -> Result<Vec<u8>> {
    if let Ok(Some(bin_idx)) = crate::token_bin_index::TokenBinIndex::open(ob_dir, tokenizer) {
        if bin_idx.is_fresh() {
            return Ok(bin_idx.generate_forget_set());
        }
    }

    let entries = crate::token_index::generate_forget_set_fast(ob_dir, tokenizer)?;
    Ok(entries)
}

pub fn write_forget_set(path: &Path, bitmask: &[u8]) -> Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bitmask)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ob_dir(tmp: &TempDir) -> std::path::PathBuf {
        let ob = tmp.path().join(".ob");
        std::fs::create_dir_all(&ob).unwrap();
        tmp.path().to_path_buf()
    }

    fn write_merged_file(
        ob_dir: &Path,
        tokenizer: &str,
        file_num: u64,
        entries: &[crate::token_index::TokenIndexEntry],
    ) {
        let dir = ob_dir
            .join(".ob")
            .join(format!("token-index.{}", tokenizer));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{:03}", file_num));
        let mut file = std::fs::File::create(&path).unwrap();
        for e in entries {
            writeln!(file, "{}", serde_json::to_string(e).unwrap()).unwrap();
        }
    }

    fn entry(revoked: bool) -> crate::token_index::TokenIndexEntry {
        crate::token_index::TokenIndexEntry {
            token_count: 1,
            sources: vec!["src".to_string()],
            tokenizer: "gpt2".to_string(),
            revoked,
        }
    }

    #[test]
    fn test_generate_forget_set_with_revoked() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let entries = vec![
            entry(false),
            entry(true),
            entry(false),
            entry(true),
            entry(true),
            entry(false),
            entry(false),
            entry(true),
            entry(false),
        ];

        write_merged_file(&ob_dir, "gpt2", 0, &entries);

        let bitmask = generate_forget_set(&ob_dir, "gpt2").unwrap();
        assert_eq!(bitmask.len(), 2);

        // revoked at indices 1,3,4,7 → byte 0 = 0b10011010
        assert_eq!(bitmask[0], 0b10011010);
        // index 8 is active → byte 1 = 0
        assert_eq!(bitmask[1], 0b00000000);
    }

    #[test]
    fn test_generate_forget_set_empty() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let bitmask = generate_forget_set(&ob_dir, "gpt2").unwrap();
        assert!(bitmask.is_empty());
    }

    #[test]
    fn test_generate_forget_set_all_revoked() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = make_ob_dir(&tmp);

        let entries = vec![entry(true), entry(true), entry(true)];

        write_merged_file(&ob_dir, "gpt2", 0, &entries);

        let bitmask = generate_forget_set(&ob_dir, "gpt2").unwrap();
        assert_eq!(bitmask.len(), 1);

        assert_eq!(bitmask[0], 0b00000111);
    }

    #[test]
    fn test_write_forget_set_creates_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("forget.bin");

        let bitmask: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF];
        write_forget_set(&path, &bitmask).unwrap();

        let read_back = std::fs::read(&path).unwrap();
        assert_eq!(read_back, bitmask);
    }
}

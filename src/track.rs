use std::path::Path;

pub fn track(
    ob_dir: &Path,
    data: &serde_json::Value,
    file: &str,
    sources: &[String],
    token_count: Option<u64>,
    tokenizer: Option<&str>,
) -> anyhow::Result<String> {
    let line_hash = crate::hash::compute_hash(data);
    crate::indexer::index_document(ob_dir, &line_hash, file, sources)?;

    if let (Some(tc), Some(tok)) = (token_count, tokenizer) {
        crate::token_index::write_pid(ob_dir, tok, tc, sources)?;
    }

    Ok(line_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn test_track_with_token_index() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = tmp.path().to_path_buf();

        std::fs::create_dir_all(ob_dir.join(".ob").join("document-index")).unwrap();

        let data = json!({"content": "test line"});
        let sources = vec!["abc123".to_string(), "def456".to_string()];
        let token_count = 42;
        let tokenizer = "gpt2";

        let line_hash = track(
            &ob_dir,
            &data,
            "test.txt",
            &sources,
            Some(token_count),
            Some(tokenizer),
        )
        .unwrap();

        assert!(!line_hash.is_empty());

        let manifest_bucket = ob_dir.join(".ob").join("document-index").join(&line_hash[..2]);
        assert!(manifest_bucket.exists());

        let pid_files = crate::token_index::list_pid_files(&ob_dir, tokenizer).unwrap();
        assert_eq!(pid_files.len(), 1);

        let records = crate::storage::jsonl_read(&pid_files[0]).unwrap();
        assert_eq!(records.len(), 1);
        let entry: crate::token_index::TokenIndexEntry =
            serde_json::from_value(records[0].clone()).unwrap();
        assert_eq!(entry.token_count, token_count);
        assert_eq!(entry.sources, sources);
        assert_eq!(entry.tokenizer, tokenizer);
        assert!(!entry.revoked);
    }

    #[test]
    fn test_track_without_token_params() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = tmp.path().to_path_buf();

        std::fs::create_dir_all(ob_dir.join(".ob").join("document-index")).unwrap();

        let data = json!({"content": "test line"});
        let sources = vec!["abc123".to_string()];

        let line_hash = track(
            &ob_dir,
            &data,
            "test.txt",
            &sources,
            None,
            None,
        )
        .unwrap();

        assert!(!line_hash.is_empty());

        let manifest_bucket = ob_dir.join(".ob").join("document-index").join(&line_hash[..2]);
        assert!(manifest_bucket.exists());

        let gpt2_files = crate::token_index::list_pid_files(&ob_dir, "gpt2").unwrap();
        assert!(gpt2_files.is_empty());
    }

    #[test]
    fn test_track_partial_token_params() {
        let tmp = TempDir::new().unwrap();
        let ob_dir = tmp.path().to_path_buf();

        std::fs::create_dir_all(ob_dir.join(".ob").join("document-index")).unwrap();

        let data = json!({"content": "test line"});
        let sources = vec!["abc123".to_string()];

        let line_hash1 = track(
            &ob_dir,
            &data,
            "test1.txt",
            &sources,
            Some(42),
            None,
        )
        .unwrap();

        let line_hash2 = track(
            &ob_dir,
            &data,
            "test2.txt",
            &sources,
            None,
            Some("gpt2"),
        )
        .unwrap();

        assert!(!line_hash1.is_empty());
        assert!(!line_hash2.is_empty());

        let gpt2_files = crate::token_index::list_pid_files(&ob_dir, "gpt2").unwrap();
        assert!(gpt2_files.is_empty());
    }
}

use anyhow::Result;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

pub struct MergeResult {
    pub authors_added: usize,
    pub sections_added: usize,
    pub document_added: usize,
    pub token_index_added: usize,
    pub skipped: usize,
}

pub fn absorb(source: &Path, target: &Path) -> Result<MergeResult> {
    let source_ob = source.join(".ob");
    anyhow::ensure!(
        source_ob.is_dir(),
        "Source has no .ob/ directory: {}",
        source.display()
    );

    let mut result = MergeResult {
        authors_added: 0,
        sections_added: 0,
        document_added: 0,
        token_index_added: 0,
        skipped: 0,
    };

    let (a, s1) = absorb_authors(source, target)?;
    result.authors_added = a;
    result.skipped += s1;

    let (s, s2) = absorb_sections(source, target)?;
    result.sections_added = s;
    result.skipped += s2;

    let (m, s3) = absorb_manifest(source, target)?;
    result.document_added = m;
    result.skipped += s3;

    let (ti, s4) = absorb_token_index(source, target)?;
    result.token_index_added = ti;
    result.skipped += s4;

    // Rebuild binary indices after successful merge so indexed queries
    // reflect the new data immediately.
    if result.authors_added + result.sections_added + result.document_added > 0 {
        let _ = crate::index::build_index(target);
    }

    crate::oplog::append(
        target,
        "merge",
        &format!(
            "source={} authors={} sections={} document={} token_index={} skipped={}",
            source.display(),
            result.authors_added,
            result.sections_added,
            result.document_added,
            result.token_index_added,
            result.skipped
        ),
    )?;

    Ok(result)
}

fn iterate_layer(ob_dir: &Path, layer: &str) -> Result<Vec<serde_json::Value>> {
    let dir = ob_dir.join(".ob").join(layer);
    let mut records = Vec::new();
    if !dir.is_dir() {
        return Ok(records);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.path().is_file() {
            continue;
        }
        records.extend(crate::storage::jsonl_read(&entry.path())?);
    }
    Ok(records)
}

fn absorb_authors(source: &Path, target: &Path) -> Result<(usize, usize)> {
    let mut existing_ids: HashSet<String> = HashSet::new();
    for rec in iterate_layer(target, "authors")? {
        if let Some(id) = rec.get("id").and_then(|v| v.as_str()) {
            existing_ids.insert(id.to_string());
        }
    }

    let mut added = 0;
    let mut skipped = 0;
    for rec in iterate_layer(source, "authors")? {
        let id = match rec.get("id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                skipped += 1;
                continue;
            }
        };
        if existing_ids.contains(&id) {
            skipped += 1;
            continue;
        }
        let shard = crate::storage::shard_path(target, "authors", &id);
        crate::storage::jsonl_append(&shard, &rec)?;
        existing_ids.insert(id);
        added += 1;
    }
    Ok((added, skipped))
}

fn absorb_sections(source: &Path, target: &Path) -> Result<(usize, usize)> {
    let mut existing: HashSet<String> = HashSet::new();
    for rec in iterate_layer(target, "sections")? {
        if let Some(sh) = rec.get("section_hash").and_then(|v| v.as_str()) {
            existing.insert(sh.to_string());
        }
    }

    let mut added = 0;
    let mut skipped = 0;
    for rec in iterate_layer(source, "sections")? {
        let sh = match rec.get("section_hash").and_then(|v| v.as_str()) {
            Some(sh) => sh.to_string(),
            None => {
                skipped += 1;
                continue;
            }
        };
        if existing.contains(&sh) {
            skipped += 1;
            continue;
        }
        let shard = crate::storage::shard_path(target, "sections", &sh);
        crate::storage::jsonl_append(&shard, &rec)?;
        existing.insert(sh);
        added += 1;
    }
    Ok((added, skipped))
}

fn absorb_manifest(source: &Path, target: &Path) -> Result<(usize, usize)> {
    let mut existing: HashSet<(String, String)> = HashSet::new();
    for rec in iterate_layer(target, "document-index")? {
        let lh = rec.get("line_hash").and_then(|v| v.as_str());
        let f = rec.get("file").and_then(|v| v.as_str());
        if let (Some(lh), Some(f)) = (lh, f) {
            existing.insert((lh.to_string(), f.to_string()));
        }
    }

    let mut added = 0;
    let mut skipped = 0;
    for rec in iterate_layer(source, "document-index")? {
        let lh = rec.get("line_hash").and_then(|v| v.as_str());
        let f = rec.get("file").and_then(|v| v.as_str());
        match (lh, f) {
            (Some(lh), Some(f)) => {
                let key = (lh.to_string(), f.to_string());
                if existing.contains(&key) {
                    skipped += 1;
                    continue;
                }
                let shard = crate::storage::shard_path(target, "document-index", lh);
                crate::storage::jsonl_append(&shard, &rec)?;
                existing.insert(key);
                added += 1;
            }
             _ => {
                 skipped += 1;
             }
         }
     }
     Ok((added, skipped))
 }

fn absorb_token_index(source: &Path, target: &Path) -> Result<(usize, usize)> {
    let tokenizers = crate::token_index::list_tokenizers(source)?;
    let mut total_added = 0usize;
    let mut total_skipped = 0usize;

    for tokenizer in &tokenizers {
        let source_entries = crate::token_index::read_merged(source, tokenizer)?;
        let target_entries = crate::token_index::read_merged(target, tokenizer)?;

        let mut existing: HashSet<(u64, Vec<String>, String)> = HashSet::new();
        for entry in &target_entries {
            let mut sorted_sources = entry.sources.clone();
            sorted_sources.sort();
            existing.insert((entry.token_count, sorted_sources, entry.tokenizer.clone()));
        }

        let merged_dir = target
            .join(".ob")
            .join(format!("token-index.{}", tokenizer));
        std::fs::create_dir_all(&merged_dir)?;

        let mut max_num: u64 = 0;
        for entry in std::fs::read_dir(&merged_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(num) = name.parse::<u64>() {
                max_num = max_num.max(num);
            }
        }

        let file_num = max_num + 1;
        let mut current_file: Option<std::fs::File> = None;

        for entry in &source_entries {
            let mut sorted_sources = entry.sources.clone();
            sorted_sources.sort();
            let key = (entry.token_count, sorted_sources, entry.tokenizer.clone());

            if existing.contains(&key) {
                total_skipped += 1;
                continue;
            }

            if current_file.is_none() {
                let path = merged_dir.join(format!("{:03}", file_num));
                current_file = Some(std::fs::File::create(&path)?);
            }

            let line = serde_json::to_string(&entry)?;
            if let Some(ref mut f) = current_file {
                writeln!(f, "{}", line)?;
            }

            existing.insert(key);
            total_added += 1;
        }
    }

    Ok((total_added, total_skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_token_entries(
        dir: &Path,
        tokenizer: &str,
        file_num: u64,
        entries: &[crate::token_index::TokenIndexEntry],
    ) {
        let merged_dir = dir.join(".ob").join(format!("token-index.{}", tokenizer));
        std::fs::create_dir_all(&merged_dir).unwrap();
        let path = merged_dir.join(format!("{:03}", file_num));
        let mut file = std::fs::File::create(&path).unwrap();
        for e in entries {
            writeln!(file, "{}", serde_json::to_string(e).unwrap()).unwrap();
        }
    }

    #[test]
    fn test_absorb_token_index_merges_entries() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob")).unwrap();
        std::fs::create_dir_all(target.join(".ob")).unwrap();

        let entries = vec![crate::token_index::TokenIndexEntry {
            token_count: 42,
            sources: vec!["src_a".to_string()],
            tokenizer: "gpt2".to_string(),
            revoked: false,
        }];
        write_token_entries(source, "gpt2", 0, &entries);

        let (added, skipped) = absorb_token_index(source, target).unwrap();
        assert_eq!(added, 1);
        assert_eq!(skipped, 0);

        let merged = crate::token_index::read_merged(target, "gpt2").unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].token_count, 42);
    }

    #[test]
    fn test_absorb_token_index_skips_duplicates() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob")).unwrap();
        std::fs::create_dir_all(target.join(".ob")).unwrap();

        let entry = crate::token_index::TokenIndexEntry {
            token_count: 10,
            sources: vec!["src_x".to_string()],
            tokenizer: "llama".to_string(),
            revoked: false,
        };
        write_token_entries(source, "llama", 0, &[entry.clone()]);
        write_token_entries(target, "llama", 0, &[entry]);

        let (added, skipped) = absorb_token_index(source, target).unwrap();
        assert_eq!(added, 0);
        assert_eq!(skipped, 1);

        let merged = crate::token_index::read_merged(target, "llama").unwrap();
        assert_eq!(merged.len(), 1);
    }

    fn write_jsonl(dir: &Path, layer: &str, shard: &str, records: &[serde_json::Value]) {
        let path = dir.join(".ob").join(layer).join(shard);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        for rec in records {
            writeln!(file, "{}", serde_json::to_string(rec).unwrap()).unwrap();
        }
    }

    fn read_jsonl_layer(dir: &Path, layer: &str) -> Vec<serde_json::Value> {
        iterate_layer(dir, layer).unwrap()
    }

    #[test]
    fn test_absorb_authors_merges_new_author() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob/authors")).unwrap();
        std::fs::create_dir_all(target.join(".ob/authors")).unwrap();

        let author = serde_json::json!({
            "id": "abcd1234",
            "name": "Alice",
            "email": "alice@example.com"
        });
        write_jsonl(source, "authors", "ab", &[author]);

        let (added, skipped) = absorb_authors(source, target).unwrap();
        assert_eq!(added, 1);
        assert_eq!(skipped, 0);

        let merged = read_jsonl_layer(target, "authors");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["id"], "abcd1234");
    }

    #[test]
    fn test_absorb_authors_skips_existing() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob/authors")).unwrap();
        std::fs::create_dir_all(target.join(".ob/authors")).unwrap();

        let author = serde_json::json!({
            "id": "abcd1234",
            "name": "Alice",
            "email": "alice@example.com"
        });
        write_jsonl(source, "authors", "ab", &[author.clone()]);
        write_jsonl(target, "authors", "ab", &[author]);

        let (added, skipped) = absorb_authors(source, target).unwrap();
        assert_eq!(added, 0);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn test_absorb_sections_merges_new_section() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob/sections")).unwrap();
        std::fs::create_dir_all(target.join(".ob/sections")).unwrap();

        let section = serde_json::json!({
            "section_hash": "face5678",
            "path": "data.jsonl",
            "authors": ["abcd1234"],
            "license": "MIT"
        });
        write_jsonl(source, "sections", "fa", &[section]);

        let (added, skipped) = absorb_sections(source, target).unwrap();
        assert_eq!(added, 1);
        assert_eq!(skipped, 0);

        let merged = read_jsonl_layer(target, "sections");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["section_hash"], "face5678");
    }

    #[test]
    fn test_absorb_sections_skips_duplicate_hash() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob/sections")).unwrap();
        std::fs::create_dir_all(target.join(".ob/sections")).unwrap();

        let section = serde_json::json!({
            "section_hash": "face5678",
            "path": "data.jsonl",
            "authors": ["abcd1234"],
            "license": "MIT"
        });
        write_jsonl(source, "sections", "fa", &[section.clone()]);
        write_jsonl(target, "sections", "fa", &[section]);

        let (added, skipped) = absorb_sections(source, target).unwrap();
        assert_eq!(added, 0);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn test_absorb_manifest_merges_new_record() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob/document-index")).unwrap();
        std::fs::create_dir_all(target.join(".ob/document-index")).unwrap();

        let manifest = serde_json::json!({
            "line_hash": "deadbeef",
            "file": "train.jsonl",
            "sources": ["face5678"]
        });
        write_jsonl(source, "document-index", "de", &[manifest]);

        let (added, skipped) = absorb_manifest(source, target).unwrap();
        assert_eq!(added, 1);
        assert_eq!(skipped, 0);

        let merged = read_jsonl_layer(target, "document-index");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["line_hash"], "deadbeef");
    }

    #[test]
    fn test_absorb_manifest_skips_duplicate_line_hash_file() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob/document-index")).unwrap();
        std::fs::create_dir_all(target.join(".ob/document-index")).unwrap();

        let manifest = serde_json::json!({
            "line_hash": "deadbeef",
            "file": "train.jsonl",
            "sources": ["face5678"]
        });
        write_jsonl(source, "document-index", "de", &[manifest.clone()]);
        write_jsonl(target, "document-index", "de", &[manifest]);

        let (added, skipped) = absorb_manifest(source, target).unwrap();
        assert_eq!(added, 0);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn test_absorb_full_end_to_end() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        // Source has full provenance chain
        std::fs::create_dir_all(source.join(".ob/authors")).unwrap();
        std::fs::create_dir_all(source.join(".ob/sections")).unwrap();
        std::fs::create_dir_all(source.join(".ob/document-index")).unwrap();

        write_jsonl(source, "authors", "ab", &[serde_json::json!({
            "id": "ab1234", "name": "Bob", "email": "bob@x.com"
        })]);
        write_jsonl(source, "sections", "cd", &[serde_json::json!({
            "section_hash": "cd5678", "path": "raw.txt", "authors": ["ab1234"]
        })]);
        write_jsonl(source, "document-index", "ef", &[serde_json::json!({
            "line_hash": "ef9012", "file": "train.jsonl", "sources": ["cd5678"]
        })]);

        // Target has one existing author and empty layers
        std::fs::create_dir_all(target.join(".ob/authors")).unwrap();
        std::fs::create_dir_all(target.join(".ob/sections")).unwrap();
        std::fs::create_dir_all(target.join(".ob/document-index")).unwrap();
        write_jsonl(target, "authors", "ab", &[serde_json::json!({
            "id": "ab1234", "name": "Bob", "email": "bob@x.com"
        })]);

        let result = absorb(source, target).unwrap();

        assert_eq!(result.authors_added, 0); // duplicate
        assert_eq!(result.sections_added, 1);
        assert_eq!(result.document_added, 1);
        assert_eq!(result.skipped, 1); // the duplicate author

        // Verify data landed in correct shards
        let authors = read_jsonl_layer(target, "authors");
        assert_eq!(authors.len(), 1); // not duplicated
        let sections = read_jsonl_layer(target, "sections");
        assert_eq!(sections.len(), 1);
        let manifests = read_jsonl_layer(target, "document-index");
        assert_eq!(manifests.len(), 1);
    }

    #[test]
    fn test_absorb_idempotent() {
        let source_tmp = tempfile::TempDir::new().unwrap();
        let target_tmp = tempfile::TempDir::new().unwrap();
        let source = source_tmp.path();
        let target = target_tmp.path();

        std::fs::create_dir_all(source.join(".ob/authors")).unwrap();
        std::fs::create_dir_all(target.join(".ob/authors")).unwrap();

        write_jsonl(source, "authors", "ab", &[serde_json::json!({
            "id": "ab1234", "name": "Alice", "email": "alice@x.com"
        })]);

        let r1 = absorb(source, target).unwrap();
        assert_eq!(r1.authors_added, 1);

        let r2 = absorb(source, target).unwrap();
        assert_eq!(r2.authors_added, 0);
        assert_eq!(r2.skipped, 1);

        let authors = read_jsonl_layer(target, "authors");
        assert_eq!(authors.len(), 1);
    }
}

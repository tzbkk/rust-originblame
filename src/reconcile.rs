use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Default)]
pub struct ReconcileResult {
    pub hash_matched: usize,
    pub semantic_matched: usize,
    pub new_lines: usize,
    pub orphans: usize,
    pub errors: usize,
    pub orphan_hashes: Vec<String>,
    pub duration_ms: f64,
}

fn compute_line_hash(line: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
        crate::hash::compute_hash(&val)
    } else {
        crate::hash::compute_hash(&serde_json::json!(line))
    }
}

fn load_manifest_index(
    ob_dir: &Path,
    file: &str,
) -> Result<HashMap<String, Vec<serde_json::Value>>> {
    let all = crate::storage::shard_iterate_all(ob_dir, "document-index")?;
    let mut index: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for rec in all {
        let rec_file = rec.get("file").and_then(|v| v.as_str()).unwrap_or("");
        if rec_file != file {
            continue;
        }
        let revoked = rec
            .get("revoked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if revoked {
            continue;
        }
        if let Some(lh) = rec.get("line_hash").and_then(|v| v.as_str()) {
            index.entry(lh.to_string()).or_default().push(rec);
        }
    }
    Ok(index)
}

pub fn reconcile(
    ob_dir: &Path,
    data_file: &Path,
    _model: Option<&str>,
    _threshold: f64,
    _embedding_api: Option<&str>,
    _compute_all_embeddings: bool,
) -> Result<ReconcileResult> {
    let start = std::time::Instant::now();

    let pid_files = crate::indexer::list_pid_files(ob_dir)?;
    anyhow::ensure!(
        pid_files.is_empty(),
        "Unmerged PID files found. Run `ob clean` first."
    );

    let file_str = data_file.to_string_lossy().to_string();

    let mmap = crate::mmap_lines::mmap_file(data_file)?
        .ok_or_else(|| anyhow::anyhow!("File not found: {}", data_file.display()))?;

    let mut manifest_index = load_manifest_index(ob_dir, &file_str)?;

    let mut result = ReconcileResult::default();

    for line_bytes in crate::mmap_lines::iter_lines(&mmap) {
        let line = match std::str::from_utf8(line_bytes) {
            Ok(s) => s,
            Err(_) => {
                result.errors += 1;
                continue;
            }
        };

        let line_hash = compute_line_hash(line);

        if let Some(records) = manifest_index.remove(&line_hash) {
            result.hash_matched += records.len();
        } else {
            result.new_lines += 1;
        }
    }

    for (hash, records) in &manifest_index {
        result.orphans += records.len();
        result.orphan_hashes.push(hash.clone());

        for rec in records {
            let orphan_rec = match rec.clone() {
                serde_json::Value::Object(mut map) => {
                    map.insert("orphan".to_string(), serde_json::Value::Bool(true));
                    serde_json::Value::Object(map)
                }
                other => other,
            };
            if let Some(lh) = orphan_rec.get("line_hash").and_then(|v| v.as_str()) {
                let shard = crate::storage::shard_path(ob_dir, "document-index", lh);
                crate::storage::jsonl_append(&shard, &orphan_rec)?;
            }
        }
    }

    result.duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(result)
}

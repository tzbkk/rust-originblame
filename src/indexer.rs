use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRecord {
    pub line_hash: String,
    pub file: String,
    pub sources: Vec<String>,
}

pub fn index_document(ob_dir: &Path, line_hash: &str, file: &str, sources: &[String], source_type: &str) -> Result<()> {
    let bucket = &line_hash[..2];
    let shard_path = ob_dir.join(".ob").join("document-index").join(bucket);
    let record = serde_json::json!({
        "line_hash": line_hash,
        "file": file,
        "sources": sources,
        "source_type": source_type,
        "revoked": false,
    });
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&shard_path)?
        .write_all(format!("{}\n", serde_json::to_string(&record)?).as_bytes())?;
    Ok(())
}

pub fn lookup_document(ob_dir: &Path, line_hash: &str) -> Result<Vec<DocumentRecord>> {
    let bucket = &line_hash[..2];
    let shard_path = ob_dir.join(".ob").join("document-index").join(bucket);
    let mmap = match crate::mmap_lines::mmap_file(&shard_path)? {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };
    let mut results = Vec::new();
    for line in crate::mmap_lines::iter_lines(&mmap) {
        if let Ok(m) = serde_json::from_slice::<DocumentRecord>(line) {
            if m.line_hash == line_hash {
                results.push(m);
            }
        }
    }
    Ok(results)
}

pub fn read_all_documents(ob_dir: &Path) -> Result<Vec<serde_json::Value>> {
    let records = crate::storage::shard_iterate_all(ob_dir, "document-index")?;
    let mut manifests = Vec::new();
    for rec in records {
        if serde_json::from_value::<DocumentRecord>(rec.clone()).is_ok() {
            manifests.push(rec);
        }
    }
    Ok(manifests)
}

pub fn list_pid_files(ob_dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let ob_dot = ob_dir.join(".ob");
    if !ob_dot.is_dir() {
        return Ok(Vec::new());
    }
    let mut pid_files = Vec::new();
    for entry in std::fs::read_dir(&ob_dot)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(suffix) = name.strip_prefix("docidx.") {
            if suffix.len() != 2 || !suffix.chars().all(|c| c.is_ascii_hexdigit()) {
                if entry.path().is_file() {
                    pid_files.push(entry.path());
                }
            }
        }
    }
    Ok(pid_files)
}

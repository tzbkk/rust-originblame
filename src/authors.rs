use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub revoked: bool,
}

pub fn author_id(name: &str, email: &str) -> String {
    crate::hash::compute_hash(&serde_json::json!({
        "name": name,
        "email": email
    }))
}

pub fn add(ob_dir: &Path, name: &str, email: &str) -> Result<Author> {
    let id = author_id(name, email);
    let author = Author {
        id,
        name: name.to_string(),
        email: email.to_string(),
        revoked: false,
    };
    let bucket = &author.id[..2];
    let shard_path = ob_dir.join(".ob").join("authors").join(bucket);
    let line = serde_json::to_string(&author)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&shard_path)
        .with_context(|| format!("opening shard {:?}", shard_path))?
        .write_all(format!("{}\n", line).as_bytes())?;
    Ok(author)
}

fn scan_bucket(
    path: &Path,
    name: Option<&str>,
    email: Option<&str>,
    results: &mut Vec<Author>,
) -> Result<()> {
    let file = std::fs::File::open(path)?;
    let mmap = unsafe { memmap2::Mmap::map(&file)? };
    for line in mmap.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if let Ok(author) = serde_json::from_slice::<Author>(line) {
            let name_match = name.map_or(true, |n| author.name == n);
            let email_match = email.map_or(true, |e| author.email == e);
            if name_match && email_match {
                results.push(author);
            }
        }
    }
    Ok(())
}

pub fn query(ob_dir: &Path, name: Option<&str>, email: Option<&str>) -> Result<Vec<Author>> {
    let mut results = Vec::new();
    let authors_dir = ob_dir.join(".ob").join("authors");
    if !authors_dir.is_dir() {
        return Ok(results);
    }
    if let (Some(n), Some(e)) = (name, email) {
        let id = author_id(n, e);
        let bucket = &id[..2];
        let bucket_path = authors_dir.join(bucket);
        if bucket_path.is_file() {
            scan_bucket(&bucket_path, Some(n), Some(e), &mut results)?;
        }
    } else {
        let entries: Vec<_> = std::fs::read_dir(&authors_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();
        results = entries
            .par_iter()
            .flat_map(|entry| {
                let mut local = Vec::new();
                let file = match std::fs::File::open(entry.path()) {
                    Ok(f) => f,
                    Err(_) => return Vec::new(),
                };
                let mmap = match unsafe { memmap2::Mmap::map(&file) } {
                    Ok(m) => m,
                    Err(_) => return Vec::new(),
                };
                for line in mmap.split(|&b| b == b'\n') {
                    let line = line.strip_suffix(b"\r").unwrap_or(line);
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(author) = serde_json::from_slice::<Author>(line) {
                        let name_match = name.map_or(true, |n| author.name == n);
                        let email_match = email.map_or(true, |e| author.email == e);
                        if name_match && email_match {
                            local.push(author);
                        }
                    }
                }
                local
            })
            .collect();
    }
    Ok(results)
}

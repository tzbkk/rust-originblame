use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub section_hash: String,
    pub path: String,
    pub authors: Vec<String>,
    pub contributors: Vec<String>,
    pub license: String,
    pub year: String,
}

pub fn register_section(
    ob_dir: &Path,
    path: &str,
    authors: &[String],
    contributors: &[String],
    license: &str,
    year: &str,
) -> Result<Section> {
    let section_hash = crate::hash::compute_hash(&serde_json::json!({
        "path": path,
        "authors": authors,
        "contributors": contributors,
        "license": license,
        "year": year
    }));
    let section = Section {
        section_hash,
        path: path.to_string(),
        authors: authors.to_vec(),
        contributors: contributors.to_vec(),
        license: license.to_string(),
        year: year.to_string(),
    };
    let bucket = &section.section_hash[..2];
    let shard_path = ob_dir.join(".ob").join("sections").join(bucket);
    let line = serde_json::to_string(&section)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&shard_path)?
        .write_all(format!("{}\n", line).as_bytes())?;
    Ok(section)
}

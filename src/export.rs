use anyhow::Result;
use std::path::Path;

pub fn export_copyright(
    ob_dir: &Path,
    output: Option<&Path>,
    data_files: Option<&[String]>,
) -> Result<String> {
    let pid_files = crate::indexer::list_pid_files(ob_dir)?;
    anyhow::ensure!(
        pid_files.is_empty(),
        "Unmerged PID files found. Run `ob clean` first."
    );

    let manifests = crate::indexer::read_all_documents(ob_dir)?;
    let sections = crate::storage::shard_iterate_all(ob_dir, "sections")?;
    let authors = crate::storage::shard_iterate_all(ob_dir, "authors")?;

    let mut section_map = std::collections::HashMap::new();
    for s in &sections {
        if let Some(hash) = s.get("section_hash").and_then(|v| v.as_str()) {
            section_map.insert(hash.to_string(), s.clone());
        }
    }

    let mut author_map = std::collections::HashMap::new();
    for a in &authors {
        if let Some(id) = a.get("id").and_then(|v| v.as_str()) {
            author_map.insert(id.to_string(), a.clone());
        }
    }

    let mut blocks = Vec::new();
    for m in &manifests {
        let file_val = m.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let line_hash = m.get("line_hash").and_then(|v| v.as_str()).unwrap_or("");

        if let Some(filter) = data_files {
            if !filter.iter().any(|f| f == file_val) {
                continue;
            }
        }

        let revoked = m.get("revoked").and_then(|v| v.as_bool()).unwrap_or(false);
        if revoked {
            continue;
        }

        let sources = match m.get("sources").and_then(|v| v.as_array()) {
            Some(s) => s,
            None => continue,
        };

        for src in sources {
            let section_hash = match src.as_str() {
                Some(s) => s,
                None => continue,
            };
            let section = match section_map.get(section_hash) {
                Some(s) => s,
                None => continue,
            };
            let section_revoked = section
                .get("revoked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if section_revoked {
                continue;
            }

            let license = section
                .get("license")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let year = section.get("year").and_then(|v| v.as_str()).unwrap_or("");

            if let Some(section_authors) = section.get("authors").and_then(|v| v.as_array()) {
                for aid in section_authors {
                    let author_id = match aid.as_str() {
                        Some(s) => s,
                        None => continue,
                    };
                    let author = match author_map.get(author_id) {
                        Some(a) => a,
                        None => continue,
                    };
                    let author_revoked = author
                        .get("revoked")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if author_revoked {
                        continue;
                    }
                    let name = author
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");
                    let email = author.get("email").and_then(|v| v.as_str()).unwrap_or("");

                    let mut block = String::new();
                    block.push_str(&format!("Files: {}:{}\n", file_val, line_hash));
                    if !year.is_empty() {
                        block.push_str(&format!("Copyright: {} {} <{}>\n", year, name, email));
                    } else {
                        block.push_str(&format!("Copyright: {} <{}>\n", name, email));
                    }
                    block.push_str(&format!("License: {}\n", license));
                    blocks.push(block);
                }
            }
        }
    }

    let result = blocks.join("\n");
    if let Some(output_path) = output {
        std::fs::write(output_path, &result)?;
    }
    Ok(result)
}

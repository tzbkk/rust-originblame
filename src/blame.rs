use std::path::Path;

pub fn blame(
    ob_dir: &Path,
    _file: &str,
    line_content: &str,
) -> anyhow::Result<Vec<crate::indexer::DocumentRecord>> {
    let data: serde_json::Value =
        serde_json::from_str(line_content).unwrap_or_else(|_| serde_json::json!(line_content));
    let line_hash = crate::hash::compute_hash(&data);
    crate::indexer::lookup_document(ob_dir, &line_hash)
}

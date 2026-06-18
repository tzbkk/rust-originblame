//! PyO3 bindings — expose all Rust functions as the `_ob_native` Python module.

#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use similar::{capture_diff_slices, Algorithm};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_err(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

fn json_to_py(py: Python<'_>, v: serde_json::Value) -> PyObject {
    match v {
        serde_json::Value::Null => py.None(),
        serde_json::Value::Bool(b) => b.into_pyobject(py).unwrap().to_owned().into(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py).unwrap().to_owned().into()
            } else {
                n.as_f64().unwrap().into_pyobject(py).unwrap().to_owned().into()
            }
        }
        serde_json::Value::String(s) => s.into_pyobject(py).unwrap().to_owned().into(),
        serde_json::Value::Array(arr) => {
            let list = pyo3::types::PyList::empty(py);
            for item in arr {
                list.append(json_to_py(py, item)).unwrap();
            }
            list.into()
        }
        serde_json::Value::Object(map) => {
            let dict = pyo3::types::PyDict::new(py);
            for (k, v) in map {
                dict.set_item(k, json_to_py(py, v)).unwrap();
            }
            dict.into()
        }
    }
}

fn py_to_json(obj: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    let json_mod = obj.py().import("json")?;
    let dumps = json_mod.call_method1("dumps", (obj,))?;
    let s: String = dumps.extract()?;
    serde_json::from_str(&s).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

// ---------------------------------------------------------------------------
// Core API (api.py level)
// ---------------------------------------------------------------------------

#[pyfunction]
fn init(ob_dir: &str) -> PyResult<()> {
    let path = Path::new(ob_dir);
    crate::ObDir::new(path).init().map_err(to_err)
}

#[pyfunction]
fn author_add(name: &str, email: &str, ob_dir: &str) -> PyResult<String> {
    let path = Path::new(ob_dir);
    crate::authors::add(path, name, email)
        .map(|a| a.id)
        .map_err(to_err)
}

#[pyfunction]
fn author_query(ob_dir: &str, name: Option<&str>, email: Option<&str>) -> PyResult<Vec<String>> {
    let path = Path::new(ob_dir);
    let authors = crate::authors::query(path, name, email).map_err(to_err)?;
    Ok(authors
        .into_iter()
        .map(|a| serde_json::to_string(&a).unwrap())
        .collect())
}

#[pyfunction]
fn register_section(
    path: &str,
    authors: Vec<String>,
    contributors: Vec<String>,
    license: &str,
    year: &str,
    ob_dir: &str,
) -> PyResult<String> {
    let ob_path = Path::new(ob_dir);
    crate::register::register_section(ob_path, path, &authors, &contributors, license, year)
        .map(|s| s.section_hash)
        .map_err(to_err)
}

#[pyfunction]
#[pyo3(signature = (data, file, sources, ob_dir, token_count=None, tokenizer=None))]
fn track(
    data: &Bound<'_, PyAny>,
    file: &str,
    sources: Vec<String>,
    ob_dir: &str,
    token_count: Option<u64>,
    tokenizer: Option<&str>,
) -> PyResult<String> {
    let ob_path = Path::new(ob_dir);
    let value = py_to_json(data)?;
    crate::track::track(ob_path, &value, file, &sources, token_count, tokenizer)
        .map_err(to_err)
}

// ---------------------------------------------------------------------------
// Indexer
// ---------------------------------------------------------------------------

#[pyfunction]
fn index_document(ob_dir: &str, line_hash: &str, file: &str, sources: Vec<String>) -> PyResult<()> {
    crate::indexer::index_document(Path::new(ob_dir), line_hash, file, &sources).map_err(to_err)
}

#[pyfunction]
fn lookup_document(py: Python<'_>, ob_dir: &str, line_hash: &str) -> PyResult<Vec<PyObject>> {
    let results = crate::indexer::lookup_document(Path::new(ob_dir), line_hash).map_err(to_err)?;
    Ok(results
        .into_iter()
        .map(|m| json_to_py(py, serde_json::to_value(m).unwrap()))
        .collect())
}

#[pyfunction]
fn read_all_documents(py: Python<'_>, ob_dir: &str) -> PyResult<Vec<PyObject>> {
    let results = crate::indexer::read_all_documents(Path::new(ob_dir)).map_err(to_err)?;
    Ok(results.into_iter().map(|v| json_to_py(py, v)).collect())
}

// ---------------------------------------------------------------------------
// Token Index
// ---------------------------------------------------------------------------

#[pyfunction]
fn token_index_write_pid(
    ob_dir: &str,
    tokenizer: &str,
    token_count: u64,
    sources: Vec<String>,
) -> PyResult<()> {
    crate::token_index::write_pid(Path::new(ob_dir), tokenizer, token_count, &sources)
        .map_err(to_err)
}

#[pyfunction]
fn token_index_merge_pid_files(ob_dir: &str, tokenizer: &str) -> PyResult<(usize, usize)> {
    crate::token_index::merge_pid_files(Path::new(ob_dir), tokenizer).map_err(to_err)
}

#[pyfunction]
fn token_index_list_tokenizers(ob_dir: &str) -> PyResult<Vec<String>> {
    crate::token_index::list_tokenizers(Path::new(ob_dir)).map_err(to_err)
}

#[pyfunction]
fn token_index_token_status(py: Python<'_>, ob_dir: &str, tokenizer: &str) -> PyResult<PyObject> {
    let status = crate::token_index::token_status(Path::new(ob_dir), tokenizer).map_err(to_err)?;
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("total_entries", status.total_entries)?;
    dict.set_item("active_entries", status.active_entries)?;
    dict.set_item("revoked_entries", status.revoked_entries)?;
    dict.set_item("total_tokens", status.total_tokens)?;
    Ok(dict.into())
}

#[pyfunction]
fn token_index_revoke_by_sources(
    ob_dir: &str,
    tokenizer: &str,
    target_sources: HashSet<String>,
) -> PyResult<usize> {
    crate::token_index::revoke_by_sources(Path::new(ob_dir), tokenizer, &target_sources)
        .map_err(to_err)
}

#[pyfunction]
fn token_index_build_binary_index(ob_dir: &str, tokenizer: &str) -> PyResult<String> {
    let stats = crate::token_index::build_binary_index(Path::new(ob_dir), tokenizer)
        .map_err(to_err)?;
    Ok(format!(
        "entries={}, sources={}",
        stats.entry_count, stats.unique_source_count
    ))
}

// ---------------------------------------------------------------------------
// Blame
// ---------------------------------------------------------------------------

#[pyfunction]
fn blame(py: Python<'_>, ob_dir: &str, file: &str, line_content: &str) -> PyResult<Vec<PyObject>> {
    let results = crate::blame::blame(Path::new(ob_dir), file, line_content).map_err(to_err)?;
    Ok(results
        .into_iter()
        .map(|m| json_to_py(py, serde_json::to_value(m).unwrap()))
        .collect())
}

// ---------------------------------------------------------------------------
// Show
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Blame diff (line-level diff for revision attribution)
// ---------------------------------------------------------------------------

/// Compute line-level diff opcodes for blame attribution.
///
/// Returns (tag, i1, i2, j1, j2) tuples compatible with
/// difflib.SequenceMatcher.get_opcodes().
#[pyfunction]
fn blame_diff(
    old_lines: Vec<String>,
    new_lines: Vec<String>,
) -> Vec<(String, usize, usize, usize, usize)> {
    let ops = capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines);

    let mut result: Vec<(String, usize, usize, usize, usize)> = Vec::new();

    for op in ops {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        let tag_str = match tag {
            similar::DiffTag::Equal => "equal",
            similar::DiffTag::Delete => "delete",
            similar::DiffTag::Insert => "insert",
            similar::DiffTag::Replace => "replace",
        };
        result.push((
            tag_str.to_string(),
            old_range.start,
            old_range.end,
            new_range.start,
            new_range.end,
        ));
    }

    result
}

// ---------------------------------------------------------------------------
// Show
// ---------------------------------------------------------------------------

#[pyfunction]
fn show_by_author(py: Python<'_>, ob_dir: &str, author_name: &str) -> PyResult<Vec<PyObject>> {
    let results = crate::show::show_by_author(Path::new(ob_dir), author_name).map_err(to_err)?;
    Ok(results
        .into_iter()
        .map(|m| json_to_py(py, serde_json::to_value(m).unwrap()))
        .collect())
}

#[pyfunction]
fn show_by_author_token(py: Python<'_>, ob_dir: &str, author_name: &str, tokenizer: &str) -> PyResult<PyObject> {
    let result =
        crate::show::show_by_author_token(Path::new(ob_dir), author_name, tokenizer)
            .map_err(to_err)?;
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("total_tokens", result.total_tokens)?;
    dict.set_item("document_count", result.document_count)?;
    let entries: Vec<PyObject> = result
        .entries
        .into_iter()
        .map(|e| json_to_py(py, serde_json::to_value(e).unwrap()))
        .collect();
    dict.set_item("entries", entries)?;
    Ok(dict.into())
}

#[pyfunction]
fn show_by_section(py: Python<'_>, ob_dir: &str, section_hash: &str) -> PyResult<Vec<PyObject>> {
    let results = crate::show::show_by_section(Path::new(ob_dir), section_hash).map_err(to_err)?;
    Ok(results
        .into_iter()
        .map(|m| json_to_py(py, serde_json::to_value(m).unwrap()))
        .collect())
}

#[pyfunction]
fn show_by_license(py: Python<'_>, ob_dir: &str, license: &str) -> PyResult<Vec<PyObject>> {
    let results = crate::show::show_by_license(Path::new(ob_dir), license).map_err(to_err)?;
    Ok(results
        .into_iter()
        .map(|m| json_to_py(py, serde_json::to_value(m).unwrap()))
        .collect())
}

// ---------------------------------------------------------------------------
// Revoke
// ---------------------------------------------------------------------------

#[pyfunction]
#[pyo3(signature = (ob_dir, author_name, reverse=false))]
fn revoke_by_author(ob_dir: &str, author_name: &str, reverse: bool) -> PyResult<usize> {
    crate::revoke::revoke_by_author(Path::new(ob_dir), author_name, reverse).map_err(to_err)
}

#[pyfunction]
#[pyo3(signature = (ob_dir, section_hash, reverse=false))]
fn revoke_section(ob_dir: &str, section_hash: &str, reverse: bool) -> PyResult<usize> {
    crate::revoke::revoke_section(Path::new(ob_dir), section_hash, reverse).map_err(to_err)
}

#[pyfunction]
#[pyo3(signature = (ob_dir, author_name, tokenizer, reverse=false))]
fn revoke_by_author_token(ob_dir: &str, author_name: &str, tokenizer: &str, reverse: bool) -> PyResult<usize> {
    crate::revoke::revoke_by_author_token(Path::new(ob_dir), author_name, tokenizer, reverse)
        .map_err(to_err)
}

#[pyfunction]
#[pyo3(signature = (ob_dir, line_hash, file, reverse=false))]
fn revoke_manifest(ob_dir: &str, line_hash: &str, file: &str, reverse: bool) -> PyResult<usize> {
    crate::revoke::revoke_manifest(Path::new(ob_dir), line_hash, file, reverse).map_err(to_err)
}

// ---------------------------------------------------------------------------
// Purge
// ---------------------------------------------------------------------------

#[pyfunction]
fn purge_revoked(py: Python<'_>, ob_dir: &str, data_file: &str, dry_run: bool) -> PyResult<PyObject> {
    let result = crate::purge::purge_revoked(Path::new(ob_dir), Path::new(data_file), dry_run)
        .map_err(to_err)?;
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("purged", result.purged)?;
    dict.set_item("kept", result.kept)?;
    dict.set_item("archived", result.archived)?;
    Ok(dict.into())
}

#[pyfunction]
fn purge_by_author_indexd(
    py: Python<'_>,
    ob_dir: &str,
    author_name: &str,
    data_file: &str,
    dry_run: bool,
) -> PyResult<PyObject> {
    let result = crate::purge::purge_by_author_indexed(
        Path::new(ob_dir),
        author_name,
        Path::new(data_file),
        dry_run,
    )
    .map_err(to_err)?;
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("purged", result.purged)?;
    dict.set_item("kept", result.kept)?;
    dict.set_item("archived", result.archived)?;
    Ok(dict.into())
}

#[pyfunction]
fn purge_reverse(ob_dir: &str, data_file: &str) -> PyResult<usize> {
    crate::purge::purge_reverse(Path::new(ob_dir), Path::new(data_file)).map_err(to_err)
}

// ---------------------------------------------------------------------------
// Clean
// ---------------------------------------------------------------------------

#[pyfunction]
fn clean(py: Python<'_>, ob_dir: &str, split: bool) -> PyResult<PyObject> {
    let result = crate::clean::clean(Path::new(ob_dir), split).map_err(to_err)?;
    let dict = pyo3::types::PyDict::new(py);
        dict.set_item("document_merged", result.document_merged)?;
    dict.set_item("pid_files_deleted", result.pid_files_deleted)?;
    dict.set_item("archived_records", result.archived_records)?;
    dict.set_item("log_rotated", result.log_rotated)?;
    dict.set_item("token_index_merged", result.token_index_merged)?;
    dict.set_item("token_index_pid_deleted", result.token_index_pid_deleted)?;
    Ok(dict.into())
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

#[pyfunction]
fn merge_absorb(py: Python<'_>, source: &str, target: &str) -> PyResult<PyObject> {
    let result = crate::merge::absorb(Path::new(source), Path::new(target)).map_err(to_err)?;
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("authors_added", result.authors_added)?;
    dict.set_item("sections_added", result.sections_added)?;
        dict.set_item("document_added", result.document_added)?;
    dict.set_item("token_index_added", result.token_index_added)?;
    dict.set_item("skipped", result.skipped)?;
    Ok(dict.into())
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

#[pyfunction]
fn build_index(py: Python<'_>, ob_dir: &str) -> PyResult<PyObject> {
    let result = crate::index::build_index(Path::new(ob_dir)).map_err(to_err)?;
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("authors", result.authors)?;
    dict.set_item("sections", result.sections)?;
    dict.set_item("total", result.total)?;
    dict.set_item("token_index_entries", result.token_index_entries)?;
    Ok(dict.into())
}

// ---------------------------------------------------------------------------
// Generate Set
// ---------------------------------------------------------------------------

#[pyfunction]
fn generate_forget_set(ob_dir: &str, tokenizer: &str) -> PyResult<Vec<u8>> {
    crate::generate_set::generate_forget_set(Path::new(ob_dir), tokenizer).map_err(to_err)
}

#[pyfunction]
fn write_forget_set(path: &str, bitmask: Vec<u8>) -> PyResult<()> {
    crate::generate_set::write_forget_set(Path::new(path), &bitmask).map_err(to_err)
}

// ---------------------------------------------------------------------------
// Storage (low-level, used by _native_compat.py)
// ---------------------------------------------------------------------------

#[pyfunction]
fn jsonl_read(py: Python<'_>, path: &str) -> PyResult<Vec<PyObject>> {
    let records = crate::storage::jsonl_read(Path::new(path)).map_err(to_err)?;
    Ok(records.into_iter().map(|v| json_to_py(py, v)).collect())
}

#[pyfunction]
fn jsonl_append(path: &str, record: &Bound<'_, PyAny>) -> PyResult<()> {
    let value = py_to_json(record)?;
    crate::storage::jsonl_append(Path::new(path), &value).map_err(to_err)
}

#[pyfunction]
fn shard_path_str(ob_dir: &str, layer: &str, hash_hex: &str) -> PyResult<String> {
    Ok(crate::storage::shard_path(Path::new(ob_dir), layer, hash_hex)
        .to_string_lossy()
        .to_string())
}

#[pyfunction]
fn shard_read(py: Python<'_>, ob_dir: &str, layer: &str, hash_hex: &str) -> PyResult<Vec<PyObject>> {
    let records = crate::storage::shard_read(Path::new(ob_dir), layer, hash_hex).map_err(to_err)?;
    Ok(records.into_iter().map(|v| json_to_py(py, v)).collect())
}

#[pyfunction]
fn shard_iterate_all(py: Python<'_>, ob_dir: &str, layer: &str) -> PyResult<Vec<PyObject>> {
    let records = crate::storage::shard_iterate_all(Path::new(ob_dir), layer).map_err(to_err)?;
    Ok(records.into_iter().map(|v| json_to_py(py, v)).collect())
}

// ---------------------------------------------------------------------------
// Oplog
// ---------------------------------------------------------------------------

#[pyfunction]
fn oplog_append(ob_dir: &str, operation: &str, detail: &str) -> PyResult<()> {
    crate::oplog::append(Path::new(ob_dir), operation, detail).map_err(to_err)
}

// ---------------------------------------------------------------------------
// Hash
// ---------------------------------------------------------------------------

#[pyfunction]
fn compute_hash(data: &Bound<'_, PyAny>) -> PyResult<String> {
    let value = py_to_json(data)?;
    Ok(crate::hash::compute_hash(&value))
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

#[pymodule]
fn _ob_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(init, m)?)?;
    m.add_function(wrap_pyfunction!(author_add, m)?)?;
    m.add_function(wrap_pyfunction!(author_query, m)?)?;
    m.add_function(wrap_pyfunction!(register_section, m)?)?;
    m.add_function(wrap_pyfunction!(track, m)?)?;

    m.add_function(wrap_pyfunction!(index_document, m)?)?;
    m.add_function(wrap_pyfunction!(lookup_document, m)?)?;
    m.add_function(wrap_pyfunction!(read_all_documents, m)?)?;

    m.add_function(wrap_pyfunction!(token_index_write_pid, m)?)?;
    m.add_function(wrap_pyfunction!(token_index_merge_pid_files, m)?)?;
    m.add_function(wrap_pyfunction!(token_index_list_tokenizers, m)?)?;
    m.add_function(wrap_pyfunction!(token_index_token_status, m)?)?;
    m.add_function(wrap_pyfunction!(token_index_revoke_by_sources, m)?)?;
    m.add_function(wrap_pyfunction!(token_index_build_binary_index, m)?)?;

    m.add_function(wrap_pyfunction!(blame, m)?)?;
    m.add_function(wrap_pyfunction!(blame_diff, m)?)?;
    m.add_function(wrap_pyfunction!(show_by_author, m)?)?;
    m.add_function(wrap_pyfunction!(show_by_author_token, m)?)?;
    m.add_function(wrap_pyfunction!(show_by_section, m)?)?;
    m.add_function(wrap_pyfunction!(show_by_license, m)?)?;
    m.add_function(wrap_pyfunction!(revoke_by_author, m)?)?;
    m.add_function(wrap_pyfunction!(revoke_section, m)?)?;
    m.add_function(wrap_pyfunction!(revoke_by_author_token, m)?)?;
    m.add_function(wrap_pyfunction!(purge_revoked, m)?)?;
    m.add_function(wrap_pyfunction!(purge_by_author_indexd, m)?)?;
    m.add_function(wrap_pyfunction!(purge_reverse, m)?)?;
    m.add_function(wrap_pyfunction!(clean, m)?)?;
    m.add_function(wrap_pyfunction!(merge_absorb, m)?)?;
    m.add_function(wrap_pyfunction!(build_index, m)?)?;
    m.add_function(wrap_pyfunction!(generate_forget_set, m)?)?;
    m.add_function(wrap_pyfunction!(write_forget_set, m)?)?;

    m.add_function(wrap_pyfunction!(jsonl_read, m)?)?;
    m.add_function(wrap_pyfunction!(jsonl_append, m)?)?;
    m.add_function(wrap_pyfunction!(shard_path_str, m)?)?;
    m.add_function(wrap_pyfunction!(shard_read, m)?)?;
    m.add_function(wrap_pyfunction!(shard_iterate_all, m)?)?;
    m.add_function(wrap_pyfunction!(oplog_append, m)?)?;
    m.add_function(wrap_pyfunction!(compute_hash, m)?)?;

    Ok(())
}

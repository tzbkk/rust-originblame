# rust-originblame

Rust native implementation of [OriginBlame](https://github.com/tzbkk/originblame) — record- and token-level data provenance for AI training datasets.

## Why Rust

The Python package's pure-Python fallback scans lines in ~580ms. The Rust native implementation does it in <0.2ms using mmap, rayon parallelism, and a binary index — up to 2,900× faster on show/purge at scale. When the Rust binary is available (built via `cargo build --release` or on `PATH`), the Python package automatically delegates to it.

## Build

Requires Rust 1.85+ (edition 2024).

```bash
cargo build --release
./target/release/ob version
```

## Python Package

Install from PyPI:

```bash
pip install originblame
```

Or from source:

```bash
cd python
pip install .                        # Core package (pure Python + Rust backend if binary found)
pip install -e ".[dev]"              # Development with pytest + ruff
```

When the Rust `ob` binary is available (built via `cargo build --release` or on `PATH`), the Python package automatically delegates performance-critical operations to it. Pure-Python fallback is used when the binary is not found.

**Delegation architecture**: The Python package uses a 3-tier fallback chain:

1. **`_ob_native`** (PyO3 cdylib) — fastest path, compiled from `src/python.rs` with 38 PyO3 bindings
2. **`ob` binary** (subprocess) — fallback if PyO3 module not built
3. **Pure Python** — last resort, no native dependencies

All CLI commands now delegate to the PyO3 bindings directly, avoiding subprocess overhead.

Build with PyO3 support: `maturin develop --release` (requires `pip install maturin`).

```python
from ob import init, author_add, register_section, source, track

init()
author_add("Wikimedia", "wikimedia@example.com")
register_section("raw/wiki.xml", ["wikimedia@example.com"], "CC-BY-SA-4.0", "2024")

# Use source.append with optional section filtering
source.append("raw/wiki.xml")  # all sections for this file
source.append("raw/wiki.xml", section="abc123...")  # only one section

# Use track with source= parameter
track(data, "data.jsonl", source="raw/wiki.xml")  # explicit file path
track(data, "data.jsonl", source=["abc123..."])  # explicit section hashes
track(data, "data.jsonl")  # use source stack (backward compatible)
```

Optional utilities (parsers, embedding reconciliation):

```bash
pip install ./python/packages/ob-util
pip install ./python/packages/ob-util[reconcile]   # with torch + sentence-transformers
```

### Python API Reference

All functions are importable from the top-level `ob` package:

```python
from ob import init, author_add, register_section, track, source
from ob.exceptions import ODError  # base exception
```

#### `init(force=False, ob_dir=None)`
Initialize a `.ob/` tracking directory in `ob_dir` (defaults to cwd). Raises `OBInitError` if `.ob/` exists but is invalid.

#### `author_add(name, email, ob_dir=None) -> str`
Register an author. Returns the 64-char SHA-256 `author_id` (hash of name+email).

#### `register_section(path, authors, license, year, contributors=None, ob_dir=None) -> str`
Register a section linking a file path to authors and license. `authors` is a list of names/emails to resolve. Returns the 64-char `section_hash`.

#### `track(data, file, embedding=None, model=None, *, source=None, ob_dir=None) -> TrackResult`
Record a provenance entry for `data` (dict or str) in `file`.
- `source`: file path (resolves to all sections for that file), list of section hashes, or `None` (uses the source stack).
- `embedding`/`model`: optional embedding vector + model name (must be provided together).
- Returns a `TrackResult` with `.line_hash`, `.file`, `.sources`, `.written` (False if idempotent skip).

#### Source stack: `source.append()` / `source.pop()`
Thread-local stack for implicit source assignment:
```python
source.append("raw/wiki.xml")        # push all sections for this file
source.append("raw/wiki.xml", section="abc123...")  # push one section
track(data, "data.jsonl")            # uses source stack
source.pop()                         # pop top entry

# Context manager (auto push/pop):
from ob.source import sources
with sources("raw/wiki.xml"):
    track(data, "data.jsonl")
```

#### Exceptions
All errors inherit from `ODError`:
`OBInitError`, `OBNotInitializedError`, `OBAuthorError`, `OBSectionError`, `OBSourceError`, `OBTrackError`, `OBStorageError`, `OBRevokeError`, `OBPurgeError`, `OBCleanError`, `OBMergeError`.

## CLI Reference

```
ob init [PATH]                          Initialize .ob/ tracking directory
ob author.add NAME EMAIL                Register an author
ob register.add --path P --authors A     Register a section
  --license LICENSE --year YEAR

ob blame FILE LINE                      Show which section(s) a line belongs to
ob show [--author NAME] [--email EMAIL]  Show provenance metadata
  [--section HASH] [--license LICENSE]
  [--revoked] [--index] [--tokenizer T]
ob revoke --author NAME                  Revoke at author level (all lines/sections)
ob revoke --section HASH                 Revoke at section level
ob revoke --line-hash HASH --file FILE   Revoke at line level
  [--email EMAIL] [--tokenizer T]
ob purge FILE [--index] [--author NAME] [--dry-run]  Physically delete revoked data (requires prior revoke)
ob status [--tokenizer TOKENIZER]        Show summary statistics
ob clean [--split]                       Merge PID files, archive revoked records
ob merge --absorb PATH                   Absorb provenance from another repository (auto-rebuilds index)
ob generate-set --tokenizer T -o FILE   Generate binary forget set (bitmask)
ob log [--op OP] [--since TIMESTAMP]   Query operation audit log (Python path)

ob index build                          Build binary index (OBIDXF02)

ob reconcile FILE [-m MODEL]            Reconcile provenance after data edits
  [-t THRESHOLD] [-e EMBEDDING_API]
  [--compute-all-embeddings]
ob export-copyright [-o FILE]           Export DEP-5 copyright file
  [--data-file FILE]
ob parse --parser FORMAT --file FILE    Parse structured data (MediaWiki XML)
  [--license LICENSE] [--split]
ob version                              Show version
```

### Token-Level Provenance

Add `--tokenizer NAME` to `show`, `revoke`, `status` to query token-index entries:

```bash
ob show --author InternetArchiveBot --tokenizer gpt2
# → 144480919 tokens across 43332 documents by InternetArchiveBot (tokenizer: gpt2)

ob revoke --author InternetArchiveBot --tokenizer gpt2
ob generate-set --tokenizer gpt2 -o forget.bin
```

The token-index operates independently of the document-index. No `data.jsonl` required — provenance is recorded during the tokenization/pack stage.

## Architecture

```
.ob/
  authors/              who: name, email, id = sha256(name+email)
  sections/            what: file path + authors + license, sharded by sha256
  document-index/      which line came from where: (line_hash, file, sources)
  token-index.gpt2/     per-document token counts with sources (per tokenizer)
  index/                binary index: OBIDXF02 with type-tagged IndexRef
  log                   operation audit trail
```

Three layers: **authors ← sections ← document-index**. Extended with an independent **token-index** layer for streaming pipelines.

All operations are logged to `.ob/log` (JSONL audit trail). Use `ob log` (Python path) to query; Rust binary delegates this to the Python package.

### Binary Index (OBIDXF02)

Type-tagged references in the index:

| Tag | Type | Content |
|-----|------|---------|
| `0x00` | DocumentIndexShard | bucket prefix byte |
| `0x01` | TokenIndexRange | tokenizer, file number, byte offset, length |

A single section can reference both document-index shards and token-index ranges across multiple tokenizers.

### Token-Index Entry

```json
{"token_count": 5, "sources": ["a3f7b2..."], "tokenizer": "gpt2", "revoked": false}
```

- `token_count`: tokens produced from one document
- `sources`: section hashes linking to the author chain
- `tokenizer`: identifier string (e.g., "gpt2", "llama3")
- `revoked`: boolean, marks entry as revoked

### Forget Set Generation

`ob generate-set` produces a binary bitmask where each bit corresponds to one token-index entry (1 = revoked, 0 = active). Directly usable by unlearning algorithms (NPO, exact retraining, etc.).

## Modules

| File | Purpose |
|------|---------|
| `show.rs` | Show provenance (line + token level, revoked, --section, --license, --revoked, --index) |
| `token_index.rs` | Token-index storage (PID files, merge, query) |
| `token_bin_index.rs` | OBIDXTI01 token binary index |
| `main.rs` | CLI entry point, command dispatch |
| `merge.rs` | Merge/absorb from another repository with auto index rebuild |
| `binary_index.rs` | OBIDXF02 binary index with type-tagged IndexRef |
| `python.rs` | PyO3 bindings (_ob_native module) |
| `purge.rs` | Physically delete revoked data |
| `revoke.rs` | Revoke at 3 levels (author, section, line) |
| `index.rs` | Index building, scanning token-index files |
| `clean.rs` | Merge PID files, archive revoked records |
| `track.rs` | Track data lines with provenance |
| `generate_set.rs` | Binary forget set generation |
| `export.rs` | DEP-5 copyright export |
| `reconcile.rs` | Two-phase reconcile (hash + embedding) |
| `authors.rs` | Author CRUD and query operations |
| `mmap_lines.rs` | mmap-based file reading and line iteration |
| `storage.rs` | JSONL read/append/mmap utilities |
| `indexer.rs` | Document-index CRUD (lookup, index) |
| `lib.rs` | Library root (test utilities, shared config) |
| `register.rs` | Section registration logic |
| `embeddings.rs` | Embedding storage/retrieval |
| `hash.rs` | SHA-256 content addressing |
| `oplog.rs` | Operation audit log |
| `blame.rs` | Line-level blame lookup |

## Performance

See the [OriginBlame README](https://github.com/tzbkk/originblame#key-results) for full benchmark results including pipeline throughput, scalability, reconcile recovery, and machine unlearning evaluation.

## Tests

```bash
cargo test    # 71 Rust tests

# Python tests (ob-util package only; core ob delegates to Rust)
cd python && pytest packages/ob-util/tests/    # 107 tests
```

The core `ob` Python package has no standalone tests — all performance-critical operations delegate to the Rust binary which has its own test suite. The `ob-util` package (parsers, embeddings, copyright export) has its own test suite.

## License

MIT

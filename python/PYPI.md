# OriginBlame

Record- and token-level data provenance for AI training datasets.

When a data contributor requests removal, model trainers face a practical gap: unlearning algorithms require a forget set, yet no tool can locate which training records belong to a given author. OriginBlame propagates author identity through data processing pipelines and resolves revocation requests into precise forget sets via deterministic queries.

## Install

```bash
pip install originblame
```

Python >= 3.12 required. An optional native backend (compiled via maturin) accelerates performance-critical operations; without it, a pure-Python fallback is used.

## Quick Start

```bash
# 1. Initialize tracking in your dataset directory
ob init

# 2. Register an author
ob author.add "Wikimedia" "wikimedia@example.com"

# 3. Register a section (file + author + license)
ob register.add \
  --path raw/wiki.xml \
  --authors wikimedia@example.com \
  --license CC-BY-SA-4.0 \
  --year 2024
```

```python
from ob import init, author_add, register_section, source, track

init()
author_add("Wikimedia", "wikimedia@example.com")
register_section("raw/wiki.xml", ["wikimedia@example.com"], "CC-BY-SA-4.0", "2024")

# Track data lines with provenance
source.append("raw/wiki.xml")
for record in read_jsonl("data.jsonl"):
    track(record, file="data.jsonl")
source.pop()

# Or use the context manager
from ob.source import sources
with sources("raw/wiki.xml"):
    track(record, file="data.jsonl")
```

## Python API

### `init(force=False, ob_dir=None)`
Initialize a `.ob/` tracking directory. Raises `OBInitError` if `.ob/` exists but is invalid.

### `author_add(name, email, ob_dir=None) -> str`
Register an author. Returns the 64-char SHA-256 `author_id`.

### `register_section(path, authors, license, year, contributors=None, ob_dir=None) -> str`
Register a section linking a file path to authors and license. Returns the `section_hash`.

### `track(data, file, embedding=None, model=None, *, source=None, ob_dir=None) -> TrackResult`
Record a provenance entry for `data` (dict or str) in `file`.
- `source`: file path, list of section hashes, or `None` (uses source stack).
- Returns `TrackResult` with `.line_hash`, `.file`, `.sources`, `.written`.

### Source stack: `source.append()` / `source.pop()`
Thread-local stack for implicit source assignment:
```python
source.append("raw/wiki.xml")
track(data, "data.jsonl")
source.pop()
```

## CLI Reference

```
ob init [PATH]                          Initialize .ob/ tracking directory
ob author.add NAME EMAIL                Register an author
ob register.add --path P --authors A    Register a section
  --license LICENSE --year YEAR

ob blame FILE LINE                      Show which section(s) a line belongs to
ob show [--author NAME] [--email EMAIL] Show provenance metadata
  [--section HASH] [--license LICENSE]
  [--revoked] [--index] [--tokenizer T]
ob revoke --author NAME                 Revoke at author level
ob revoke --section HASH                Revoke at section level
ob revoke --line-hash HASH --file FILE  Revoke at line level
ob purge FILE [--author NAME] [--dry-run]  Physically delete revoked data
ob status [--tokenizer TOKENIZER]       Show summary statistics
ob clean                                Merge PID files, archive revoked records
ob merge --absorb PATH                  Absorb provenance from another repository
ob generate-set --tokenizer T -o FILE   Generate binary forget set (bitmask)
ob log [--op OP] [--since TIMESTAMP]    Query operation audit log

ob reconcile FILE [-m MODEL]            Reconcile provenance after data edits
  [-t THRESHOLD] [-e EMBEDDING_API]
ob export-copyright [-o FILE]           Export DEP-5 copyright file
ob parse --parser FORMAT --file FILE    Parse structured data (MediaWiki XML)
ob version                              Show version
```

### Token-Level Provenance

Add `--tokenizer NAME` to `show`, `revoke`, `status` to query token-index entries:

```bash
ob show --author InternetArchiveBot --tokenizer gpt2
ob revoke --author InternetArchiveBot --tokenizer gpt2
ob generate-set --tokenizer gpt2 -o forget.bin
```

## Architecture

```
.ob/
  authors/              who: name, email, id = sha256(name+email)
  sections/             what: file path + authors + license
  document-index/       which line came from where
  token-index.gpt2/     per-document token counts (per tokenizer)
  index/                binary index for fast lookups
  log                   operation audit trail
```

Three layers: **authors <- sections <- document-index**, extended with an independent **token-index** for streaming pipelines.

## License

MIT

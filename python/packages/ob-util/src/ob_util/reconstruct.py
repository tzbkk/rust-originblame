"""Retroactive provenance recovery — rebuild .ob/ from source data.

Provides hash-based exact matching (SHA-256) and fuzzy matching (character-level
Jaccard similarity, line-hash Jaccard) to recover provenance after data
transformation. Includes threshold sweeping and PR curve generation for
evaluation.

This is a best-effort retroactive tool. For real-time provenance tracking,
use the pipeline-level ``ob track`` / ``ob register`` workflow.
"""

from __future__ import annotations

import hashlib
import json
import random
from dataclasses import dataclass, field
from pathlib import Path


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def jaccard_chars(a: str, b: str) -> float:
    """Character-level Jaccard similarity: |A∩B| / |A∪B|."""
    sa, sb = set(a), set(b)
    if not sa and not sb:
        return 1.0
    if not sa or not sb:
        return 0.0
    return len(sa & sb) / len(sa | sb)


def line_hash_jaccard(a: str, b: str) -> float:
    """Line-hash Jaccard: hash each line, compare sets of hashes."""
    ha = {sha256(ln) for ln in a.splitlines() if ln.strip()}
    hb = {sha256(ln) for ln in b.splitlines() if ln.strip()}
    if not ha and not hb:
        return 1.0
    if not ha or not hb:
        return 0.0
    return len(ha & hb) / len(ha | hb)


# ---------------------------------------------------------------------------
# Source index
# ---------------------------------------------------------------------------


@dataclass
class SourceRecord:
    text: str
    title: str = ""
    authors: str = "[]"
    year: str = ""
    license: str = "CC-BY-SA-4.0"
    ob_section_hash: str = ""


def build_source_index(records: list[dict]) -> dict[str, SourceRecord]:
    """Build SHA-256 index from key fields of each record."""
    index: dict[str, SourceRecord] = {}
    for rec in records:
        h = sha256(rec.get("text", ""))
        index[h] = SourceRecord(
            text=rec.get("text", ""),
            title=rec.get("title", ""),
            authors=rec.get("authors_json", rec.get("authors", "[]")),
            year=rec.get("year", ""),
            license=rec.get("license", "CC-BY-SA-4.0"),
            ob_section_hash=rec.get("ob_section_hash", ""),
        )
    return index


# ---------------------------------------------------------------------------
# Matching
# ---------------------------------------------------------------------------


def match_record(
    query_text: str,
    source_index: dict[str, SourceRecord],
    source_texts: list[str],
    source_records: list[SourceRecord],
    sim_fn=jaccard_chars,
) -> tuple[float, SourceRecord | None]:
    """Match a query record against the source index.

    Returns (confidence, matched_record) where confidence is 1.0 for
    exact SHA-256 match, or the similarity score for fuzzy matches.
    """
    h = sha256(query_text)
    if h in source_index:
        return (1.0, source_index[h])

    best_sim, best_idx = 0.0, -1
    for i, src_text in enumerate(source_texts):
        sim = sim_fn(query_text, src_text)
        if sim > best_sim:
            best_sim = sim
            best_idx = i

    if best_sim > 0.1 and best_idx >= 0:
        return (best_sim, source_records[best_idx])
    return (0.0, None)


# ---------------------------------------------------------------------------
# Mutation simulation (for evaluation)
# ---------------------------------------------------------------------------


def _mutate(rec: dict, rng: random.Random, n_subs: int) -> dict:
    text = list(rec["text"])
    chars = "abcdefghijklmnopqrstuvwxyz0123456789"
    for _ in range(n_subs):
        if text:
            pos = rng.randint(0, len(text) - 1)
            text[pos] = rng.choice(chars)
    return {**rec, "text": "".join(text)}


def _insert_lines(rec: dict, rng: random.Random, n_lines: int) -> dict:
    lines = rec["text"].splitlines()
    for _ in range(n_lines):
        pos = rng.randint(0, len(lines))
        lines.insert(pos, f"<!-- inserted {rng.randint(1000,9999)} -->")
    return {**rec, "text": "\n".join(lines)}


def _delete_lines(rec: dict, rng: random.Random, n_lines: int) -> dict:
    lines = rec["text"].splitlines()
    for _ in range(min(n_lines, len(lines) - 1)):
        if lines:
            pos = rng.randint(0, len(lines) - 1)
            lines.pop(pos)
    return {**rec, "text": "\n".join(lines)}


def simulate_mutations(records: list[dict], rng: random.Random | None = None) -> list[dict]:
    """60% verbatim, 10% light-edit, 10% line-insert, 10% line-delete, 10% heavy."""
    if rng is None:
        rng = random.Random(42)
    mutated = []
    for rec in records:
        r = rng.random()
        if r < 0.60:
            mutated.append({**rec})
        elif r < 0.70:
            mutated.append(_mutate(rec, rng, n_subs=rng.randint(1, 3)))
        elif r < 0.80:
            mutated.append(_insert_lines(rec, rng, n_lines=rng.randint(1, 3)))
        elif r < 0.90:
            mutated.append(_delete_lines(rec, rng, n_lines=rng.randint(1, 3)))
        else:
            n = max(1, int(len(rec["text"]) * 0.15))
            mutated.append(_mutate(rec, rng, n_subs=n))
    return mutated


# ---------------------------------------------------------------------------
# Threshold sweeping
# ---------------------------------------------------------------------------


@dataclass
class SweepResult:
    threshold: float
    precision: float
    recall: float
    f1: float
    tp: int
    fp: int
    fn: int
    total_positive: int


def sweep_thresholds(
    matches: list[tuple[float, bool]],
    thresholds: list[float],
) -> list[SweepResult]:
    """Sweep confidence thresholds and compute P/R/F1."""
    total_pos = sum(1 for _, is_match in matches if is_match)
    results = []
    for t in thresholds:
        tp = sum(1 for c, m in matches if c >= t and m)
        fp = sum(1 for c, m in matches if c >= t and not m)
        fn = sum(1 for c, m in matches if c < t and m)
        prec = tp / (tp + fp) if (tp + fp) else 0.0
        rec = tp / (tp + fn) if (tp + fn) else 0.0
        f1 = 2 * prec * rec / (prec + rec) if (prec + rec) else 0.0
        results.append(SweepResult(
            threshold=t, precision=prec, recall=rec, f1=f1,
            tp=tp, fp=fp, fn=fn, total_positive=total_pos,
        ))
    return results


# ---------------------------------------------------------------------------
# PR curve
# ---------------------------------------------------------------------------


def generate_pr_curve(
    sweep_results: list[SweepResult],
    output_path: Path,
    title: str = "Retroactive Provenance Recovery",
    n_label: int = 100,
) -> Path:
    """Generate a PR curve figure. Requires matplotlib (optional dependency)."""
    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        raise ImportError(
            "matplotlib is required for PR curve generation. "
            "Install with: pip install ob-util[reconstruct]"
        )

    recalls = [r.recall for r in sweep_results]
    precisions = [r.precision for r in sweep_results]
    best = max(sweep_results, key=lambda r: r.f1)

    fig, ax = plt.subplots(1, 1, figsize=(5, 3.5))
    ax.plot(recalls, precisions, "o-", color="#2563eb", markersize=4,
            linewidth=1.5, label="PR curve", zorder=3)
    ax.plot(best.recall, best.precision, "s", color="#dc2626",
            markersize=8, zorder=4,
            label=f'Optimal θ={best.threshold:.2f} (F1={best.f1:.2f})')

    for i, r in enumerate(sweep_results):
        if i % max(1, len(sweep_results) // n_label) == 0:
            ax.annotate(f'{r.threshold:.2f}',
                        (r.recall, r.precision),
                        textcoords="offset points", xytext=(4, 6),
                        fontsize=6, color="#666666")

    ax.set_xlabel("Recall", fontsize=11)
    ax.set_ylabel("Precision", fontsize=11)
    ax.set_xlim(-0.02, 1.05)
    ax.set_ylim(-0.02, 1.05)
    ax.legend(fontsize=9, loc="lower left")
    ax.grid(True, alpha=0.3)
    ax.set_title(title, fontsize=11)
    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(str(output_path), bbox_inches="tight", dpi=300)
    plt.close(fig)
    return output_path


# ---------------------------------------------------------------------------
# Data loading
# ---------------------------------------------------------------------------


def load_records(path: Path, start: int = 0, n: int | None = None) -> list[dict]:
    """Load JSONL records from path, optionally slicing."""
    records = []
    with open(path, encoding="utf-8") as f:
        for i, line in enumerate(f):
            if i < start:
                continue
            if n is not None and len(records) >= n:
                break
            records.append(json.loads(line))
    return records

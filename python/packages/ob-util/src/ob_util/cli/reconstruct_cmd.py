import typer

from ob_util.reconstruct import generate_pr_curve, load_records, sweep_thresholds

app = typer.Typer(
    invoke_without_command=True,
    help="Retroactive provenance recovery — rebuild .ob/ from source data",
)


@app.callback(invoke_without_command=True)
def reconstruct(
    data_file: str | None = typer.Argument(None, help="JSONL data file to evaluate"),
    n_records: int = typer.Option(
        100, "--n", "-n", help="Number of records to use (default: 100)"
    ),
    seed: int = typer.Option(
        42, "--seed", "-s", help="Random seed for mutation simulation"
    ),
    output_dir: str = typer.Option(
        ".",
        "--output",
        "-o",
        help="Output directory for results",
    ),
    pr_curve: bool = typer.Option(
        False, "--pr-curve", help="Generate PR curve figure (requires matplotlib)"
    ),
):
    """Run retroactive provenance recovery PoC on a JSONL dataset.

    This simulates retroactive matching: builds a source index, applies
    mutations, then tries to match the mutated data back to source records
    using SHA-256 exact match and Jaccard character similarity.
    """
    from pathlib import Path
    import json
    import random

    data_path = Path(data_file) if data_file else None
    if data_path is None or not data_path.exists():
        typer.echo(f"Error: data file not found: {data_path}", err=True)
        raise typer.Exit(1)

    out_dir = Path(output_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    rng = random.Random(seed)

    from ob_util.reconstruct import (
        SourceRecord,
        build_source_index,
        match_record,
        simulate_mutations,
    )

    typer.echo(f"Source: {data_path}")
    typer.echo(f"Records: {n_records}, Seed: {seed}")
    typer.echo()

    # Phase 1: Build source index
    typer.echo("[Phase 1] Build source index...")
    source_records_raw = load_records(data_path, start=0, n=n_records)
    source_index = build_source_index(source_records_raw)
    source_texts = [rec.text for rec in source_index.values()]
    source_records_list = list(source_index.values())
    typer.echo(f"  Source index: {len(source_index)} unique hashes from {len(source_records_raw)} records")

    # Phase 2: Simulate mutations and match
    typer.echo("[Phase 2] Simulate retroactive recovery...")
    mutated = simulate_mutations(source_records_raw, rng)
    distractors = load_records(data_path, start=n_records * 5, n=n_records // 2)

    matches: list[tuple[float, bool]] = []
    matched_output: list[dict] = []
    for rec in mutated:
        conf, matched = match_record(rec["text"], source_index, source_texts, source_records_list)
        matches.append((conf, True))
        if matched and conf > 0.5:
            matched_output.append({
                "confidence": conf,
                "found_title": rec.get("title", ""),
                "matched_title": matched.title,
                "authors": matched.authors,
                "year": matched.year,
                "license": matched.license,
            })
    for rec in distractors:
        conf, matched = match_record(rec["text"], source_index, source_texts, source_records_list)
        matches.append((conf, False))
        if matched and conf > 0.5:
            matched_output.append({
                "confidence": conf,
                "found_title": rec.get("title", ""),
                "matched_title": matched.title,
                "authors": matched.authors,
                "year": matched.year,
                "license": matched.license,
            })

    rng.shuffle(matches)
    typer.echo(f"  Ground truth: {n_records} of {len(matches)} records from source set")

    # Phase 3: Threshold sweep
    typer.echo("[Phase 3] Threshold sweep...")
    thresholds = [0.50, 0.60, 0.70, 0.75, 0.80, 0.82, 0.85, 0.88, 0.90, 0.92, 0.95, 0.99]
    sweep = sweep_thresholds(matches, thresholds)

    typer.echo(f"  {'θ':>6s}  {'P':>6s}  {'R':>6s}  {'F1':>6s}  {'TP':>4s}  {'FP':>4s}  {'FN':>4s}")
    typer.echo("  " + "-" * 42)
    for r in sweep:
        typer.echo(
            f"  {r.threshold:6.2f}  {r.precision:6.3f}  {r.recall:6.3f}  "
            f"{r.f1:6.3f}  {r.tp:4d}  {r.fp:4d}  {r.fn:4d}"
        )

    best = max(sweep, key=lambda r: r.f1)
    typer.echo()
    typer.echo(f"Optimal: θ={best.threshold:.2f} at F1-maximum "
               f"(P={best.precision:.3f}, R={best.recall:.3f}, F1={best.f1:.3f})")
    typer.echo(f"Recovery rate: {best.tp}/{n_records} = {best.tp/n_records:.1%}")

    # Phase 4: Write outputs
    typer.echo("[Phase 4] Write outputs...")
    matched_file = out_dir / "matched_records.jsonl"
    with open(matched_file, "w", encoding="utf-8") as f:
        for mr in sorted(matched_output, key=lambda x: x["confidence"], reverse=True):
            f.write(json.dumps(mr, ensure_ascii=False) + "\n")
    typer.echo(f"  Matched records: {matched_file} ({len(matched_output)} records)")

    if pr_curve:
        try:
            fig_path = out_dir / "fig-reconstruct-threshold.pdf"
            generate_pr_curve(sweep, fig_path, f"Retroactive Provenance Recovery (n={n_records}, Wikipedia)")
            typer.echo(f"  PR curve: {fig_path}")
        except ImportError as e:
            typer.echo(f"  PR curve skipped: {e}", err=True)

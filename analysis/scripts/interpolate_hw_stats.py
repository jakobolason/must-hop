"""
Densify hw_stats CSVs referenced by an experiment manifest.

For each `hw_stats` CSV in the manifest, per node_id:
  1. Insert one point between every consecutive pair of originals
     (densified x-positions: 0, 0.5, 1, 1.5, ...).
  2. In that doubled series, after every Nth row (1-indexed,
     `EXTRA_INSERT_EVERY`), insert one additional point between it and the
     next row.
  3. The y-values for the new points come from a PCHIP fit (monotone
     piecewise cubic Hermite) over the non-NaN originals. Originals are
     preserved exactly. A new point whose bracketing originals include a
     NaN stays NaN — we don't fabricate across gaps.

Outputs:
  - New CSVs in the same directory as the originals, with `_interp` appended
    to the filename stem.
  - A new experiment JSON next to the original, with `_interp` appended,
    where each run's `hw_stats` path points at the new CSV (other fields,
    including `main_stats`, are left untouched).

Usage:
    python -m analysis.scripts.interpolate_hw_stats <path/to/experiment.json>
    # or
    python analysis/scripts/interpolate_hw_stats.py <path/to/experiment.json>
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
import pandas as pd
from scipy.interpolate import PchipInterpolator

EXTRA_INSERT_EVERY = 4  # extra midpoint after every Nth row of the doubled series


def _resolve_csv_path(csv_field: str, manifest_path: Path) -> Path:
    """
    Manifests store paths like `./analysis/data/full_hw/foo.csv` relative to
    the repo root. Resolve against cwd first (matches notebook usage), then
    fall back to walking up from the manifest location.
    """
    raw = Path(csv_field)
    if raw.exists():
        return raw
    # manifest lives at <repo>/analysis/data/experiments/<name>.json
    repo_root = manifest_path.resolve().parents[3]
    candidate = repo_root / raw
    if candidate.exists():
        return candidate
    raise FileNotFoundError(f"could not locate hw_stats CSV: {csv_field}")


def _build_output_x(n: int) -> np.ndarray:
    """
    Densified x-positions for n originals.

    Stage 1: midpoints between every consecutive pair → 0, 0.5, 1, 1.5, ...
    Stage 2: after every EXTRA_INSERT_EVERY-th row (1-indexed) of that doubled
             series, insert one extra x halfway to the next row.
    """
    doubled = np.linspace(0.0, n - 1, 2 * n - 1)
    out: list[float] = []
    for i, x in enumerate(doubled):
        out.append(float(x))
        if (i + 1) % EXTRA_INSERT_EVERY == 0 and i + 1 < len(doubled):
            out.append(float((doubled[i] + doubled[i + 1]) / 2.0))
    return np.asarray(out, dtype=float)


def _interp_block(values: np.ndarray) -> np.ndarray:
    """
    PCHIP-densify a 1-D series, preserving originals and NaN-propagating
    across any new point whose bracketing originals contain a NaN.
    """
    n = len(values)
    if n < 2:
        return values.astype(float, copy=True)

    out_x = _build_output_x(n)
    out_y = np.full(len(out_x), np.nan, dtype=float)

    is_nan = np.isnan(values)
    valid = ~is_nan
    if valid.sum() >= 2:
        # extrapolate=False → NaN outside the valid x-range (e.g. before the
        # first non-NaN original), which is what we want.
        pchip = PchipInterpolator(
            np.arange(n, dtype=float)[valid],
            values[valid],
            extrapolate=False,
        )
        out_y = pchip(out_x)

    # Re-stamp constraints:
    #   - at original integer x, use the original y (preserves NaN at NaN rows).
    #   - at a new x whose bracketing originals include a NaN, force NaN.
    for j, x in enumerate(out_x):
        if float(x).is_integer():
            out_y[j] = values[int(x)]
        else:
            lo = int(np.floor(x))
            if is_nan[lo] or is_nan[lo + 1]:
                out_y[j] = np.nan

    return out_y


def _interp_node_frame(df: pd.DataFrame) -> pd.DataFrame:
    """Interpolate a per-node sub-frame. Preserves node_id/probe_id by repeat."""
    if df.empty:
        return df.copy()

    delays = _interp_block(df["hardware_delay"].to_numpy(dtype=float))
    out_len = len(delays)

    # node_id and probe_id are constant within a node block; just broadcast.
    node_id = df["node_id"].iloc[0]
    probe_id = df["probe_id"].iloc[0]
    return pd.DataFrame(
        {
            "node_id": np.repeat(node_id, out_len),
            "probe_id": np.repeat(probe_id, out_len),
            "hardware_delay": delays,
        }
    )


def interpolate_hw_csv(src: Path, dst: Path) -> tuple[int, int]:
    """
    Read src, interpolate per-node, write to dst. Returns (rows_in, rows_out).
    The order of node blocks in the output matches the order of first appearance
    in the input — same convention the original CSVs use.
    """
    df = pd.read_csv(src)
    if not {"node_id", "probe_id", "hardware_delay"}.issubset(df.columns):
        raise ValueError(
            f"{src} is missing one of node_id/probe_id/hardware_delay columns"
        )

    # Preserve original node order
    seen: list[str] = []
    for nid in df["node_id"]:
        if nid not in seen:
            seen.append(nid)

    chunks = [_interp_node_frame(df[df["node_id"] == nid]) for nid in seen]
    out_df = pd.concat(chunks, ignore_index=True)
    out_df.to_csv(dst, index=False)
    return len(df), len(out_df)


def _suffixed(path: Path, suffix: str = "_interp") -> Path:
    """foo.csv → foo_interp.csv (preserves multi-dot stems like 14.18)."""
    return path.with_name(path.stem + suffix + path.suffix)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2

    manifest_path = Path(argv[1])
    if not manifest_path.exists():
        print(f"manifest not found: {manifest_path}", file=sys.stderr)
        return 1

    manifest = json.loads(manifest_path.read_text())
    experiments = manifest.get("experiments", [])
    if not experiments:
        print("manifest contains no experiments", file=sys.stderr)
        return 1

    new_experiments = []
    for i, run in enumerate(experiments):
        hw_field = run.get("hw_stats")
        if not hw_field:
            print(f"[{i}] no hw_stats path — leaving run unchanged")
            new_experiments.append(dict(run))
            continue

        try:
            src = _resolve_csv_path(hw_field, manifest_path)
        except FileNotFoundError as e:
            print(f"[{i}] {e} — leaving run unchanged")
            new_experiments.append(dict(run))
            continue

        dst = _suffixed(src)
        try:
            n_in, n_out = interpolate_hw_csv(src, dst)
        except (pd.errors.EmptyDataError, ValueError) as e:
            print(f"[{i}] skip {src.name}: {e}")
            new_experiments.append(dict(run))
            continue

        print(f"[{i}] {src.name}  {n_in} → {n_out} rows  →  {dst.name}")

        # Rewrite the hw_stats field in the same format (./analysis/...) the
        # original manifest used, so downstream loaders behave identically.
        new_field = str(Path(hw_field).with_name(_suffixed(Path(hw_field)).name))
        new_run = dict(run)
        new_run["hw_stats"] = new_field
        new_experiments.append(new_run)

    new_manifest = dict(manifest)
    new_manifest["experiments"] = new_experiments
    if "run_id" in new_manifest:
        new_manifest["run_id"] = f"{new_manifest['run_id']}_interp"

    out_manifest = _suffixed(manifest_path, "_interp")
    out_manifest.write_text(json.dumps(new_manifest, indent=2))
    print(f"\nwrote new manifest: {out_manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

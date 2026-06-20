#!/usr/bin/env python3
"""
Average packet size (bytes) for sensor nodes and the gateway, across every
experiment under analysis/data/experiments/.

For each experiment JSON we open its referenced main_stats CSV and pull:
  - node_bytes  (every row contributes one sensor-node packet)
  - gw_bytes    (only present in the older CSV schema)

Usage:
    python avg_packet_bytes.py
    python avg_packet_bytes.py --per-experiment
"""

import argparse
import json
import sys
from pathlib import Path

import pandas as pd

REPO_ROOT = Path(__file__).resolve().parents[2]
EXPERIMENTS_DIR = REPO_ROOT / "analysis" / "data" / "experiments"


def resolve_csv(raw: str) -> Path:
    return (REPO_ROOT / raw.lstrip("./")).resolve()


def load_bytes(csv_path: Path) -> tuple[pd.Series, pd.Series]:
    """Return (node_bytes, gw_bytes) Series from a main_stats CSV.

    Missing columns come back as empty Series so callers can concat blindly.
    """
    try:
        df = pd.read_csv(csv_path)
    except (FileNotFoundError, pd.errors.EmptyDataError):
        return pd.Series(dtype=float), pd.Series(dtype=float)

    node = df["node_bytes"].dropna() if "node_bytes" in df.columns else pd.Series(dtype=float)
    gw = df["gw_bytes"].dropna() if "gw_bytes" in df.columns else pd.Series(dtype=float)
    return node, gw


def iter_experiments():
    """Yield (json_name, exp_index, exp_dict) for every experiment block."""
    json_files = sorted(EXPERIMENTS_DIR.glob("*.json"))
    if not json_files:
        sys.exit(f"[error] No experiment JSONs in {EXPERIMENTS_DIR}")
    for jf in json_files:
        try:
            data = json.loads(jf.read_text())
        except json.JSONDecodeError as e:
            print(f"[warn] {jf.name}: {e}", file=sys.stderr)
            continue
        for i, exp in enumerate(data.get("experiments", [])):
            yield jf.name, i, exp


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--per-experiment", action="store_true",
                        help="Also print the mean for each experiment block")
    args = parser.parse_args()

    all_node = []
    all_gw = []
    per_exp_rows = []
    missing = 0
    seen = 0

    for json_name, idx, exp in iter_experiments():
        raw = exp.get("main_stats")
        if not raw:
            continue
        csv_path = resolve_csv(raw)
        seen += 1
        if not csv_path.exists():
            missing += 1
            continue

        node, gw = load_bytes(csv_path)
        all_node.append(node)
        all_gw.append(gw)

        if args.per_experiment:
            per_exp_rows.append({
                "json": json_name,
                "exp": idx,
                "sf": exp.get("sf"),
                "bw": exp.get("bw"),
                "nodes": len(exp.get("nodes", [])),
                "n_node_pkts": len(node),
                "node_mean": node.mean() if not node.empty else float("nan"),
                "n_gw_pkts": len(gw),
                "gw_mean": gw.mean() if not gw.empty else float("nan"),
            })

    if args.per_experiment and per_exp_rows:
        pe = pd.DataFrame(per_exp_rows)
        with pd.option_context("display.max_rows", None,
                               "display.width", 140,
                               "display.float_format", "{:.2f}".format):
            print(pe.to_string(index=False))
        print()

    node_all = pd.concat([s for s in all_node if not s.empty]) if all_node else pd.Series(dtype=float)
    gw_all = pd.concat([s for s in all_gw if not s.empty]) if all_gw else pd.Series(dtype=float)

    print(f"experiments scanned:   {seen}")
    print(f"CSVs missing locally:  {missing} (run sync_data.py to fetch)")
    print()
    print("Sensor-node packets")
    print(f"  count: {len(node_all)}")
    if not node_all.empty:
        print(f"  mean:  {node_all.mean():.2f} bytes")
        print(f"  std:   {node_all.std():.2f}")
        print(f"  min/max: {int(node_all.min())} / {int(node_all.max())}")
    print()
    print("Gateway packets (only logged in older CSV schema)")
    print(f"  count: {len(gw_all)}")
    if not gw_all.empty:
        print(f"  mean:  {gw_all.mean():.2f} bytes")
        print(f"  std:   {gw_all.std():.2f}")
        print(f"  min/max: {int(gw_all.min())} / {int(gw_all.max())}")


if __name__ == "__main__":
    main()

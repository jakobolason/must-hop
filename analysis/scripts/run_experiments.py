"""
Experiment runner for must-hop.
Builds the headless binary once, then sweeps through (SF, BW) combinations.

Configure EXPERIMENTS, NODES, KP, KI, and DURATION below, then run:
    uv run ./run_experiments.py
"""

import json
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

EXPERIMENTS = [
    {"sf": 7, "bw": 125},
    {"sf": 8, "bw": 125},
    {"sf": 9, "bw": 125},
    # {"sf": 10, "bw": 125},
    # {"sf": 11, "bw": 125},
    # {"sf": 12, "bw": 125},
]

# Nodes: list of {"node_id": "<source id>", "probe_id": "<probe serial>"}
# Passed to headless as positional args in "node_id:probe_id" format.
NODES = [
    {"node_id": "7", "probe_id": "1366:0101:000801024472"},
]

KP = "10"
KI = "40"

# Seconds per experiment run (~10 min default)
DURATION = 600

# Seconds to wait between runs for radio settling
INTER_RUN_DELAY = 30


REPO_ROOT = Path(__file__).parent.parent.parent.resolve()
HEADLESS_BIN = REPO_ROOT / "target" / "release" / "headless"

_DATA_PREFIX = "[headless:data] "


def build_headless() -> None:
    print("[runner] Building headless binary...")
    result = subprocess.run(
        ["cargo", "build", "--release", "-p", "must-dash", "--bin", "headless"],
        cwd=REPO_ROOT,
    )
    if result.returncode != 0:
        raise RuntimeError("headless build failed")
    print("[runner] Build OK.\n")


def _parse_data_paths(stderr: str) -> dict[str, str | None]:
    """Extract main_stats / hw_stats paths from headless stderr output."""
    paths: dict[str, str | None] = {"main_stats": None, "hw_stats": None}
    for line in stderr.splitlines():
        if not line.startswith(_DATA_PREFIX):
            continue
        kv = line.removeprefix(_DATA_PREFIX)
        key, _, value = kv.partition("=")
        if key in paths:
            paths[key] = value.strip()
    return paths


def run_experiment(
    sf: int, bw: int, nodes: list[dict[str, str]], duration: int
) -> dict[str, any]:
    node_args = [f"{n['node_id']}:{n['probe_id']}" for n in nodes]
    print(f"[runner] SF={sf} BW={bw} nodes={node_args} duration={duration}s")

    start = datetime.now()
    result = subprocess.run(
        [
            str(HEADLESS_BIN),
            "--sf",
            str(sf),
            "--bw",
            str(bw),
            "--kp",
            KP,
            "--ki",
            KI,
            "--duration",
            str(duration),
            *node_args,
        ],
        cwd=REPO_ROOT,
        stderr=subprocess.PIPE,
        text=True,
    )
    end = datetime.now()

    # Re-print stderr so the user still sees headless progress messages.
    if result.stderr:
        sys.stderr.write(result.stderr)

    data_paths = _parse_data_paths(result.stderr or "")

    elapsed = (end - start).total_seconds()
    status = "ok" if result.returncode == 0 else f"failed:{result.returncode}"
    print(f"[runner] → {status}  ({elapsed:.0f}s)\n")

    return {
        "sf": sf,
        "bw": bw,
        "nodes": [{"node_id": n["node_id"], "probe_id": n["probe_id"]} for n in nodes],
        "kp": KP,
        "ki": KI,
        "duration": duration,
        "started_at": start.isoformat(),
        "ended_at": end.isoformat(),
        "elapsed_s": elapsed,
        "exit_code": result.returncode,
        "main_stats": data_paths["main_stats"],
        "hw_stats": data_paths["hw_stats"],
    }


def main() -> None:
    build_headless()

    manifest_dir = REPO_ROOT / "analysis" / "data" / "experiments"
    manifest_dir.mkdir(parents=True, exist_ok=True)

    run_id = datetime.now().strftime("%Y%m%d_%H%M%S")
    manifest_path = manifest_dir / f"run_{run_id}.json"

    results = []

    for i, exp in enumerate(EXPERIMENTS):
        print(f"=== Experiment {i + 1}/{len(EXPERIMENTS)} ===")
        record = run_experiment(
            sf=exp["sf"],
            bw=exp["bw"],
            nodes=NODES,
            duration=DURATION,
        )
        results.append(record)

        # Write manifest after every run so progress is not lost on interruption
        manifest_path.write_text(
            json.dumps({"run_id": run_id, "experiments": results}, indent=2)
        )

        if i < len(EXPERIMENTS) - 1:
            print(f"[runner] Waiting {INTER_RUN_DELAY}s before next run...")
            time.sleep(INTER_RUN_DELAY)

    print("=== All experiments complete ===")
    print(f"Manifest: {manifest_path}")
    for r in results:
        flag = "success" if r["exit_code"] == 0 else "failed"
        print(f"  {flag}  SF={r['sf']} BW={r['bw']}  ({r['elapsed_s']:.0f}s)")
        if r["main_stats"]:
            print(f"          main_stats: {r['main_stats']}")
        if r["hw_stats"]:
            print(f"          hw_stats:   {r['hw_stats']}")


if __name__ == "__main__":
    main()

"""
Experiment runner for must-hop.
Builds the headless binary once, then sweeps through (SF, BW) combinations.

Configure EXPERIMENTS, NODES, KP, KI, and DURATION below, then run:
    uv run ./run_experiments.py
"""

import json
import subprocess
import time
from datetime import datetime
from pathlib import Path

EXPERIMENTS = [
    {"sf": 7,  "bw": 125},
    {"sf": 8,  "bw": 125},
    {"sf": 9,  "bw": 125},
    {"sf": 10, "bw": 125},
    {"sf": 11, "bw": 125},
    {"sf": 12, "bw": 125},
]

# Node source IDs 
NODES = ["7"]

KP = "50"
KI = "40"

# Seconds per experiment run (~10 min default)
DURATION = 600

# Seconds to wait between runs for radio settling
INTER_RUN_DELAY = 30


REPO_ROOT = Path(__file__).parent.parent.parent.resolve()
HEADLESS_BIN = REPO_ROOT / "target" / "release" / "headless"


def build_headless() -> None:
    print("[runner] Building headless binary...")
    result = subprocess.run(
        ["cargo", "build", "--release", "-p", "must-dash", "--bin", "headless"],
        cwd=REPO_ROOT,
    )
    if result.returncode != 0:
        raise RuntimeError("headless build failed")
    print("[runner] Build OK.\n")


def run_experiment(sf: int, bw: int, nodes: list[str], duration: int) -> dict[str, any]:
    print(f"[runner] SF={sf} BW={bw} nodes={nodes} duration={duration}s")

    start = datetime.now()
    result = subprocess.run(
        [
            str(HEADLESS_BIN),
            "--sf", str(sf),
            "--bw", str(bw),
            "--kp", KP,
            "--ki", KI,
            "--duration", str(duration),
            *nodes,
        ],
        cwd=REPO_ROOT,
    )
    end = datetime.now()

    elapsed = (end - start).total_seconds()
    status = "ok" if result.returncode == 0 else f"failed:{result.returncode}"
    print(f"[runner] → {status}  ({elapsed:.0f}s)\n")

    return {
        "sf": sf,
        "bw": bw,
        "nodes": nodes,
        "kp": KP,
        "ki": KI,
        "duration": duration,
        "started_at": start.isoformat(),
        "ended_at": end.isoformat(),
        "elapsed_s": elapsed,
        "exit_code": result.returncode,
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


if __name__ == "__main__":
    main()

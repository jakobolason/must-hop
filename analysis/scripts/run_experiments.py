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
from typing import Any

# EXPERIMENT 1: 6 different PI controllers
# KPKIs = [(40, 50), (10, 50), (13, 32), (13, 5), (1, 1), (100, 500)]
# KPKIs = [(0, 0)]
# EXPERIMENTS = [
#     {"sf": 5, "bw": 125, "kp": kp, "ki": ki, "tau": 10, "times": 5}
#     for (kp, ki) in KPKIs
# ]

# EXPERIMENT 2: Test controller with different spreading factors
# SFs = [5, 6, 7, 8, 9, 10, 11]
# EXPERIMENTS = [
#     {"sf": sf, "bw": 125, "kp": 40, "ki": 50, "tau": 10, "times": 15} for sf in SFs
# ]

# EXPERIMENT 3: Test different heartbeat intervals
TAUS = [20, 25, 30]
EXPERIMENTS = [
    {
        "sf": 12,
        "bw": 125,
        "kp": int(4 / (tau / 100)),
        "ki": int(5 / (tau / 100)),
        "tau": tau,
        "times": 10,
    }
    for tau in TAUS
]


# Nodes: list of {"node_id": "<source id>", "probe_id": "<probe serial>"}
# Passed to headless as positional args in "node_id:probe_id" format.
NODES = [
    {"node_id": "7", "probe_id": "1366:0101:000801024472"},
    {"node_id": "4", "probe_id": "1366:0101:000801024520"},
]


# Seconds per experiment run (~10 min default)
DURATION = 1200

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
    sf: int,
    bw: int,
    kp: int,
    ki: int,
    tau: int,
    nodes: list[dict[str, str]],
    duration: int,
) -> dict[str, Any]:
    node_args = [f"{n['node_id']}:{n['probe_id']}" for n in nodes]
    print(f"[runner] SF={sf} BW={bw} nodes={node_args} duration={duration}s")

    start = datetime.now()
    proc = subprocess.Popen(
        [
            str(HEADLESS_BIN),
            "--sf",
            str(sf),
            "--bw",
            str(bw),
            "--kp",
            str(kp),
            "--ki",
            str(ki),
            "--tau",
            str(tau),
            "--duration",
            str(duration),
            *node_args,
        ],
        cwd=REPO_ROOT,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        _, stderr = proc.communicate()
    except KeyboardInterrupt:
        # headless already received SIGINT from the terminal — give it time to
        # call shutdown_processes and clean up its children before we force-kill it.
        try:
            _ = proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.terminate()
            _ = proc.wait()
        raise
    end = datetime.now()

    # Re-print stderr so the user still sees headless progress messages.
    if stderr:
        _ = sys.stderr.write(stderr)

    data_paths = _parse_data_paths(stderr or "")

    elapsed = (end - start).total_seconds()
    status = "ok" if proc.returncode == 0 else f"failed:{proc.returncode}"
    print(f"[runner] → {status}  ({elapsed:.0f}s)\n")

    return {
        "sf": sf,
        "bw": bw,
        "nodes": [{"node_id": n["node_id"], "probe_id": n["probe_id"]} for n in nodes],
        "kp": kp,
        "ki": ki,
        "duration": duration,
        "started_at": start.isoformat(),
        "ended_at": end.isoformat(),
        "elapsed_s": elapsed,
        "exit_code": proc.returncode,
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

    expanded = [exp for exp in EXPERIMENTS for _ in range(exp.get("times", 1))]

    try:
        for i, exp in enumerate(expanded):
            print(f"== Experiment {i + 1}/{len(expanded)} ==")
            record = run_experiment(
                sf=exp["sf"],
                bw=exp["bw"],
                kp=exp["kp"],
                ki=exp["ki"],
                tau=exp["tau"],
                nodes=NODES,
                duration=DURATION,
            )
            results.append(record)

            # Write manifest after every run so progress is not lost on interruption
            _ = manifest_path.write_text(
                json.dumps({"run_id": run_id, "experiments": results}, indent=2)
            )

            if i < len(expanded) - 1:
                print(f"== Experiment {i + 1}/{len(expanded)} ==")
                print(f"[runner] Waiting {INTER_RUN_DELAY}s before next run...")
                try:
                    time.sleep(INTER_RUN_DELAY)
                except KeyboardInterrupt:
                    print("\n[runner] Interrupted during wait — stopping sweep.")
                    raise
    except KeyboardInterrupt:
        print(
            f"\n[runner] Sweep interrupted after {len(results)}/{len(expanded)} experiments."
        )

    print(
        "=== All experiments complete ==="
        if len(results) == len(expanded)
        else "=== Partial run ==="
    )
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

#!/usr/bin/env python3
"""
Sync experiment CSV files from a remote server.

Reads all experiment JSON files under analysis/data/experiments/, collects
every main_stats / hw_stats path, skips any that already exist locally, and
scp-s the rest from the remote host.

Remote credentials are read from .env (PI_USER, HOST_URL) at the repo root.
The remote base path defaults to ~/must-rs but can be overridden with --remote-base.

Usage:
    python sync_data.py
    python sync_data.py --remote-base ~/code/must-rs
    python sync_data.py --dry-run
"""

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path


# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).resolve().parents[2]
EXPERIMENTS_DIR = REPO_ROOT / "analysis" / "data" / "experiments"


def load_env(repo_root: Path) -> dict[str, str]:
    env_file = repo_root / ".env"
    if not env_file.exists():
        sys.exit(
            f"[error] .env not found at {env_file}. Copy .env.example and fill it in."
        )
    result = {}
    for line in env_file.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        result[key.strip()] = value.strip().strip('"').strip("'")
    return result


# ── Collect paths from experiment JSONs ───────────────────────────────────────


def collect_csv_paths() -> list[Path]:
    """Return all unique CSV paths referenced across every experiment JSON."""
    import json

    paths: set[Path] = set()
    json_files = sorted(EXPERIMENTS_DIR.glob("*.json"))
    if not json_files:
        sys.exit(f"[error] No experiment JSONs found in {EXPERIMENTS_DIR}")

    for jf in json_files:
        try:
            data = json.loads(jf.read_text())
        except Exception as e:
            print(f"[warn] Could not parse {jf.name}: {e}")
            continue

        for exp in data.get("experiments", []):
            for key in ("main_stats", "hw_stats"):
                raw = exp.get(key)
                if raw:
                    # Paths in JSON are like ./analysis/data/... — resolve from repo root
                    resolved = (REPO_ROOT / raw.lstrip("./")).resolve()
                    paths.add(resolved)

    return sorted(paths)


# ── Sync ──────────────────────────────────────────────────────────────────────


def sync(remote_base: str, host: str, dry_run: bool) -> None:
    csv_paths = collect_csv_paths()
    print(f"[info] {len(csv_paths)} CSV path(s) referenced across all experiment runs")

    missing = [p for p in csv_paths if not p.exists()]
    already_local = len(csv_paths) - len(missing)

    print(f"[info] {already_local} already local, {len(missing)} to fetch")

    if not missing:
        print("[info] Nothing to do.")
        return

    # Paths relative to repo root — these match the directory structure under
    # remote_base, so rsync can map them directly.
    rel_paths = [str(p.relative_to(REPO_ROOT)) for p in missing]

    if dry_run:
        print(f"[dry-run] Would rsync {len(rel_paths)} file(s) from {host}:{remote_base}/")
        for p in rel_paths:
            print(f"  {p}")
        return

    for p in missing:
        p.parent.mkdir(parents=True, exist_ok=True)

    # rsync doesn't shell-expand ~ in the source path when --files-from is
    # used, so resolve it explicitly with one fast SSH call first.
    abs_remote_base = remote_base
    if remote_base.startswith("~"):
        res = subprocess.run(["ssh", host, "echo $HOME"], capture_output=True, text=True)
        if res.returncode != 0:
            sys.exit(f"[error] Could not resolve remote home dir: {res.stderr.strip()}")
        abs_remote_base = remote_base.replace("~", res.stdout.strip(), 1)
        print(f"[info] Resolved remote base: {abs_remote_base}")

    # Write a temp file-list for rsync --files-from.
    # rsync opens a single SSH connection and fetches everything in one go,
    # avoiding the per-file connection storm that trips UFW rate limiting.
    with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as f:
        f.write("\n".join(rel_paths) + "\n")
        tmpfile = Path(f.name)

    try:
        print(f"[fetch] rsync {len(rel_paths)} file(s) via single SSH connection...")
        result = subprocess.run(
            [
                "rsync", "-avz",
                "--files-from", str(tmpfile),
                f"{host}:{abs_remote_base}/",
                str(REPO_ROOT) + "/",
            ],
            text=True,
        )
        if result.returncode == 0:
            print(f"\n[done] {len(missing)} fetched, {already_local} already local")
        else:
            print(f"\n[error] rsync exited with code {result.returncode}")
    finally:
        tmpfile.unlink(missing_ok=True)


# ── CLI ───────────────────────────────────────────────────────────────────────


def main() -> None:
    _ = parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    _ = parser.add_argument(
        "--remote-base",
        default="~/code/must-hop",
        help="Base directory of the repo on the remote host (default: ~/code/must-hop)",
    )
    _ = parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print what would be fetched without actually copying anything",
    )
    _ = parser.add_argument(
        "--host",
        default="jakobolason.dk",
        help="Override HOST_URL from .env",
    )
    _ = parser.add_argument(
        "--run",
        help="A possible run-*.json file to scp first"
    )

    args = parser.parse_args()

    env = load_env(REPO_ROOT)
    host = args.host or env.get("HOST_URL") or sys.exit("[error] HOST_URL not set in .env")

    print(f"[info] Remote: {host}:{args.remote_base}")
    print(f"[info] Local repo root: {REPO_ROOT}")

    sync(
        remote_base=args.remote_base,
        host=host,
        dry_run=args.dry_run,
    )


if __name__ == "__main__":
    main()

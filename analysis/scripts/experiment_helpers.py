import os
import re
from collections import defaultdict

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

# Matches tokens like SF9, BW125, KP40, KI50, TAU10 anywhere in the filename,
# and a trailing <N>nodes token.
_PARAM_RE = re.compile(r"_(SF|BW|KP|KI|TAU)(\d+)")
_NODES_RE = re.compile(r"_(\d+)nodes")


def parse_params_from_filename(path):
    """
    Extract experiment parameters from a main_stats / hw_stats filename like
    `hw_stats_01-06:21.02_SF9_BW125_KP40_KI50_TAU10_1nodes.csv`.

    Returns a dict with lowercase keys (sf, bw, kp, ki, tau, nodes); missing
    tokens are simply absent from the dict.
    """
    name = os.path.basename(str(path))
    out = {k.lower(): int(v) for k, v in _PARAM_RE.findall(name)}
    m = _NODES_RE.search(name)
    if m:
        out["nodes"] = int(m.group(1))
    return out


def get_pct_in_low(df):
    tau_mode = df["tau_hb_high"]
    lst = tau_mode == True
    first_index = None
    if lst.any():
        first_index = lst.idxmax()

    true_count = tau_mode.sum()
    false_count = (~tau_mode).sum()
    false_time = false_count * 10
    total_time = false_time + true_count * 30
    return (false_time / total_time, first_index)


def analyze_runs(runs, min_entries=5):
    """
    Takes a list of run dicts and returns a list of result dicts, **one per
    node per run**: a 2-node run produces two result entries (distinguishable
    via the `node_id` field). 1-node runs behave as before.

    Skips a run/node when the CSVs are missing, empty, or have fewer than
    `min_entries` rows for that node.

      sf, bw, kp, ki, tau, nodes,
      node_id,
      mean_hw_delay, std_hw_delay,
      mean_believed_err, std_believed_err,
      mean_believed_speed, std_believed_speed,
      pct_in_low (fraction 0–1),
      settling_index (first row where tau_hb_high is True, or None)
    """
    results = []
    for run in runs:
        label = f"SF={run.get('sf')} BW={run.get('bw')} started={str(run.get('started_at', '?'))[:16]}"
        try:
            stats = pd.read_csv(run["main_stats"])
            hw_stats = pd.read_csv(run["hw_stats"])
        except (pd.errors.EmptyDataError, FileNotFoundError) as e:
            print(f"  skip {label}: {e}")
            continue

        if "node_id" not in stats.columns or "node_id" not in hw_stats.columns:
            print(f"  skip {label}: missing node_id column in main_stats / hw_stats")
            continue

        # Pull params from the filename so fields not present in the run dict
        # (notably `tau` in older manifests) are still available downstream.
        fname_params = parse_params_from_filename(run["hw_stats"])
        nodes_total = len(run.get("nodes", [])) or fname_params.get("nodes")

        # Intersect node_ids that show up in both CSVs so we don't emit a row
        # for a node that has main_stats but no hardware capture (or vice versa).
        main_ids = set(stats["node_id"].unique())
        hw_ids = set(hw_stats["node_id"].unique())
        node_ids = sorted(main_ids & hw_ids)
        if not node_ids:
            print(
                f"  skip {label}: no overlap between main_stats nodes "
                f"({sorted(main_ids)}) and hw_stats nodes ({sorted(hw_ids)})"
            )
            continue
        # runs = False
        for node_id in node_ids:
            # if runs:
            # break
            # runs = True
            node_label = f"{label} node={node_id}"
            node_main = stats[stats["node_id"] == node_id]
            node_hw = hw_stats[hw_stats["node_id"] == node_id]
            if len(node_main) < min_entries or len(node_hw) < min_entries:
                print(
                    f"  skip {node_label}: too few rows "
                    f"(main={len(node_main)}, hw={len(node_hw)}, need >{min_entries})"
                )
                continue

            hw_delay = node_hw["hardware_delay"]
            mean = float(hw_delay.mean())
            std = float(hw_delay.std())
            pct_in_low, first_index = get_pct_in_low(node_main)

            # Controller's own belief about its sync error (per-packet [SYNC]).
            believed_err = (
                node_main["err_ms"]
                if "err_ms" in node_main.columns
                else pd.Series(dtype=float)
            )
            mean_believed_err = (
                float(believed_err.mean()) if not believed_err.empty else float("nan")
            )
            std_believed_err = (
                float(believed_err.std()) if not believed_err.empty else float("nan")
            )
            # PI-loop output (drift correction velocity) — the `v_s` term in
            # the controller, written as `new_speed` in main_stats.
            believed_speed = (
                node_main["new_speed"]
                if "new_speed" in node_main.columns
                else pd.Series(dtype=float)
            )
            mean_believed_speed = (
                float(believed_speed.mean())
                if not believed_speed.empty
                else float("nan")
            )
            std_believed_speed = (
                float(believed_speed.std())
                if not believed_speed.empty
                else float("nan")
            )

            results.append(
                {
                    "sf": run.get("sf", fname_params.get("sf")),
                    "bw": run.get("bw", fname_params.get("bw")),
                    "kp": run.get("kp", fname_params.get("kp")),
                    "ki": run.get("ki", fname_params.get("ki")),
                    "tau": run.get("tau", fname_params.get("tau")),
                    "nodes": nodes_total,
                    "node_id": node_id,
                    "mean_hw_delay": mean,
                    "std_hw_delay": std,
                    "mean_believed_err": mean_believed_err,
                    "std_believed_err": std_believed_err,
                    "mean_believed_speed": mean_believed_speed,
                    "std_believed_speed": std_believed_speed,
                    "pct_in_low": float(pct_in_low),
                    "settling_index": first_index,
                    "main_stats": run["main_stats"],
                    "hw_stats": run["hw_stats"],
                }
            )
    return results


def group_results(results, by=("sf", "bw")):
    """
    Group a flat results list into an ordered dict keyed by tuples of `by` values.
    e.g. group_results(results, by=('sf', 'bw'))  →  {(7, 125): [...], (8, 125): [...]}
    """
    groups = defaultdict(list)
    for r in results:
        key = tuple(r[k] for k in by)
        groups[key].append(r)
    return dict(groups)


def aggregate_group(group):
    """
    Collapse a group of repeated runs into a single summary dict.

    Mean: replaced by median + IQR — robust to the occasional outlier run
          that would otherwise pull the simple mean far off.
    Std, %InLow, Settle: mean ± std across runs.
    """
    first = group[0]
    means = np.array([r["mean_hw_delay"] for r in group])
    stds = np.array([r["std_hw_delay"] for r in group])
    plows = np.array([r["pct_in_low"] for r in group])
    settles = np.array(
        [r["settling_index"] for r in group if r["settling_index"] is not None]
    )
    q25, q75 = np.percentile(means, [25, 75])

    def _med_iqr(key):
        arr = np.array([r[key] for r in group if key in r])
        arr = arr[np.isfinite(arr)]
        if arr.size == 0:
            return float("nan"), 0.0
        b_q25, b_q75 = np.percentile(arr, [25, 75])
        return float(np.median(arr)), float(b_q75 - b_q25)

    median_belief_err, belief_err_iqr = _med_iqr("mean_believed_err")
    median_belief_speed, belief_speed_iqr = _med_iqr("mean_believed_speed")

    return {
        "sf": first["sf"],
        "bw": first["bw"],
        "kp": first["kp"],
        "ki": first["ki"],
        "tau": first.get("tau"),
        "nodes": first["nodes"],
        "node_id": first.get("node_id"),
        "n_runs": len(group),
        # Robust center: median ± IQR
        "mean_hw_delay": float(np.median(means)),
        "mean_hw_delay_iqr": float(q75 - q25),
        # Per-run Std: mean ± std across runs
        "std_hw_delay": float(np.mean(stds)),
        "std_hw_delay_sd": float(np.std(stds, ddof=1)) if len(stds) > 1 else 0.0,
        # %InLow: mean ± std across runs
        "pct_in_low": float(np.mean(plows)),
        "pct_in_low_sd": float(np.std(plows, ddof=1)) if len(plows) > 1 else 0.0,
        # Settle: median ± std across runs
        "settling_index": int(np.median(settles)) if len(settles) > 0 else None,
        "settling_sd": float(np.std(settles, ddof=1)) if len(settles) > 1 else 0.0,
        # Believed error: median ± IQR across runs of each run's mean err_ms
        "mean_believed_err": median_belief_err,
        "mean_believed_err_iqr": belief_err_iqr,
        # Believed PI-loop output speed: same aggregation, from new_speed
        "mean_believed_speed": median_belief_speed,
        "mean_believed_speed_iqr": belief_speed_iqr,
    }


_COL_W = dict(
    run=4,
    sf=4,
    bw=5,
    kp=4,
    ki=4,
    nodes=5,
    mean=18,
    std=14,
    pct=13,
    settle=11,
    belief=18,
    speed=22,
)


def _table_header(
    aggregate=False, include_believed_err=False, include_believed_speed=False
):
    cw = _COL_W
    center_label = "Median(µs)" if aggregate else "Mean(µs)"
    std_label = "Std(µs)" if aggregate else "Std(µs)"
    pct_label = "%InLow"
    settle_label = "Settle"
    head = (
        f"{'#':>{cw['run']}}  "
        f"{'SF':>{cw['sf']}}  "
        f"{'BW':>{cw['bw']}}  "
        f"{'KP':>{cw['kp']}}  "
        f"{'KI':>{cw['ki']}}  "
        f"{'Nodes':>{cw['nodes']}}  "
        f"{settle_label:>{cw['settle']}}"
        f"{center_label:>{cw['mean']}}  "
        f"{std_label:>{cw['std']}}  "
        f"{pct_label:>{cw['pct']}}  "
    )
    if include_believed_err:
        head += f"{'Belief(ms)':>{cw['belief']}}  "
    if include_believed_speed:
        head += f"{'Speed':>{cw['speed']}}  "
    return head


def _fmt_belief(r, aggregate, mean_key, iqr_key, std_key, precision):
    """Format a `mean ±spread` cell for either err or speed."""
    val = r.get(mean_key, float("nan"))
    if val != val:  # NaN
        return "—"
    if aggregate:
        iqr = r.get(iqr_key, 0.0)
        return f"${val:.{precision}f} ±{iqr:.{precision}f}i q r$,"
    std = r.get(std_key, float("nan"))
    sd_tag = f" ±{std:.{precision}f}" if std == std else ""
    return f"${val:.{precision}f}{sd_tag}$,"


def _table_row(
    i, r, aggregate=False, include_believed_err=False, include_believed_speed=False
):
    cw = _COL_W
    settle_val = r["settling_index"]

    if aggregate:
        iqr = r.get("mean_hw_delay_iqr", 0.0)
        std_sd = r.get("std_hw_delay_sd", 0.0)
        plow_sd = r.get("pct_in_low_sd", 0.0)
        s_sd = r.get("settling_sd", 0.0)

        # median ±IQR (IQR = Q75-Q25, a robust spread measure)
        mean_str = f"${r['mean_hw_delay']:.1f} ±{iqr:.1f}i q r$,"
        std_str = f"${r['std_hw_delay']:.2f} ±{std_sd:.2f}$,"
        plow_str = f"${r['pct_in_low'] * 100:.1f} ±{plow_sd * 100:.1f}%$,"
        settle_str = (
            f"${settle_val * 10} ±{s_sd * 10:.0f}$," if settle_val is not None else "—"
        )
    else:
        sd_tag = f" $±{r['mean_hw_delay_sd']:.2f}$," if "mean_hw_delay_sd" in r else ""
        mean_str = f"${r['mean_hw_delay']:.3f}{sd_tag}$,"
        std_str = f"${r['std_hw_delay']:.3f}$,"
        plow_str = f"${r['pct_in_low'] * 100:.1f}%$,"
        settle_str = str(settle_val) if settle_val is not None else "—"

    err_str = _fmt_belief(
        r,
        aggregate,
        "mean_believed_err",
        "mean_believed_err_iqr",
        "std_believed_err",
        precision=3 if not aggregate else 2,
    )
    speed_str = _fmt_belief(
        r,
        aggregate,
        "mean_believed_speed",
        "mean_believed_speed_iqr",
        "std_believed_speed",
        precision=0,
    )

    row = (
        f"{i:>{cw['run']}}  "
        f"{str(r['sf']):>{cw['sf']}}  "
        f"{str(r['bw']):>{cw['bw']}}  "
        f"{str(r['kp']):>{cw['kp']}}  "
        f"{str(r['ki']):>{cw['ki']}}  "
        f"{r['nodes']:>{cw['nodes']}}  "
        f"{settle_str:>{cw['settle']}}"
        f"{mean_str:>{cw['mean']}}  "
        f"{std_str:>{cw['std']}}  "
        f"{plow_str:>{cw['pct']}}  "
    )
    if include_believed_err:
        row += f"{err_str:>{cw['belief']}}  "
    if include_believed_speed:
        row += f"{speed_str:>{cw['speed']}}  "
    return row


def print_run_results(
    results,
    title=None,
    aggregate=False,
    include_believed_err=False,
    include_believed_speed=False,
):
    """Pretty-print a flat list of result dicts as a table."""
    header = _table_header(
        aggregate=aggregate,
        include_believed_err=include_believed_err,
        include_believed_speed=include_believed_speed,
    )
    sep = "─" * len(header)
    if title:
        print(f"\n{'── ' + title + ' ':─<{len(header)}}")
    else:
        print(sep)
    print(header)
    print(sep)
    for i, r in enumerate(results, start=1):
        print(
            _table_row(
                i,
                r,
                aggregate=aggregate,
                include_believed_err=include_believed_err,
                include_believed_speed=include_believed_speed,
            )
        )


def print_grouped_results(
    results,
    by=("sf", "bw"),
    aggregate=True,
    include_believed_err=False,
    include_believed_speed=False,
):
    """
    Group results by `by`, then print each configuration as a titled block.

    aggregate=True  (default) — one summary row per group:
                    median ±IQR for the per-run mean (robust to outliers),
                    mean ±std across runs for Std, %InLow, and Settle.
    aggregate=False — show every individual run inside each group.
    include_believed_err=True   — append a Belief(ms) column from `err_ms`
                                  (controller's view of its sync error;
                                  steady loop ⇒ near 0).
    include_believed_speed=True — append a Speed column from `new_speed`
                                  (the PI controller's drift-correction output).
    """
    grouped = group_results(results, by=by)
    for key, group in grouped.items():
        label = "  ".join(f"{k.upper()}={v}" for k, v in zip(by, key))
        if aggregate:
            print_run_results(
                [aggregate_group(group)],
                title=f"{label}  (n={len(group)})",
                aggregate=True,
                include_believed_err=include_believed_err,
                include_believed_speed=include_believed_speed,
            )
        else:
            print_run_results(
                group,
                title=label,
                include_believed_err=include_believed_err,
                include_believed_speed=include_believed_speed,
            )
        # print()


# ── 2-node experiments ────────────────────────────────────────────────────────


def analyze_two_node_runs(runs, follower_node_id, min_entries=10):
    """
    For 2-node experiments: `follower_node_id` is the solely-follower (leaf).
    The other node in each run's main_stats is the relayer (follower of GW,
    leader to the solely-follower).

    Per-run metrics:
      err_to_gw_*    — median / std of the follower's err_ms. The user's
                       convention is to treat this as the follower's error
                       relative to the GW reference.
      err_between_*  — median / std of (follower.err_ms − relayer.err_ms),
                       row-aligned by index and truncated to the shorter
                       series. This is the residual error between the two
                       nodes.

    `follower_node_id` is matched against the `node_id` column in main_stats
    (e.g. "node-7" for new CSVs, "node A" for older ones).
    """
    results = []
    for run in runs:
        label = (
            f"SF={run.get('sf')} BW={run.get('bw')} "
            f"started={str(run.get('started_at', '?'))[:16]}"
        )
        try:
            stats = pd.read_csv(run["main_stats"])
        except (pd.errors.EmptyDataError, FileNotFoundError) as e:
            print(f"  skip {label}: {e}")
            continue

        if "node_id" not in stats.columns or "err_ms" not in stats.columns:
            print(f"  skip {label}: missing node_id/err_ms columns")
            continue

        follower = stats[stats["node_id"] == follower_node_id]
        relayer = stats[stats["node_id"] != follower_node_id]

        if follower.empty:
            print(
                f"  skip {label}: no rows for follower {follower_node_id!r}; "
                f"got {sorted(stats['node_id'].unique())}"
            )
            continue
        other_ids = sorted(relayer["node_id"].unique())
        if len(other_ids) != 1:
            print(f"  skip {label}: expected exactly one relayer, got {other_ids}")
            continue
        if len(follower) < min_entries or len(relayer) < min_entries:
            print(
                f"  skip {label}: too few rows "
                f"(follower={len(follower)}, relayer={len(relayer)}, need >{min_entries})"
            )
            continue

        err_to_gw = follower["err_ms"].to_numpy()
        n = min(len(follower), len(relayer))
        err_between = (
            follower["err_ms"].iloc[:n].to_numpy()
            - relayer["err_ms"].iloc[:n].to_numpy()
        )

        fname_params = parse_params_from_filename(run["main_stats"])

        results.append(
            {
                "sf": run.get("sf", fname_params.get("sf")),
                "bw": run.get("bw", fname_params.get("bw")),
                "kp": run.get("kp", fname_params.get("kp")),
                "ki": run.get("ki", fname_params.get("ki")),
                "tau": run.get("tau", fname_params.get("tau")),
                "nodes": len(run.get("nodes", [])) or fname_params.get("nodes"),
                "follower_node_id": follower_node_id,
                "relayer_node_id": other_ids[0],
                "err_to_gw_median": float(np.median(err_to_gw)),
                "err_to_gw_std": (
                    float(np.std(err_to_gw, ddof=1)) if len(err_to_gw) > 1 else 0.0
                ),
                "err_between_median": float(np.median(err_between)),
                "err_between_std": (
                    float(np.std(err_between, ddof=1)) if len(err_between) > 1 else 0.0
                ),
                "main_stats": run["main_stats"],
            }
        )
    return results


def _aggregate_two_node_group(group):
    """Collapse repeated 2-node runs to median ±IQR across runs."""
    first = group[0]

    def _summary(key):
        arr = np.array([r[key] for r in group if np.isfinite(r[key])])
        if arr.size == 0:
            return float("nan"), 0.0
        q25, q75 = np.percentile(arr, [25, 75])
        return float(np.median(arr)), float(q75 - q25)

    err_gw_med, err_gw_iqr = _summary("err_to_gw_median")
    err_gw_std_med, _ = _summary("err_to_gw_std")
    err_bt_med, err_bt_iqr = _summary("err_between_median")
    err_bt_std_med, _ = _summary("err_between_std")

    return {
        "sf": first["sf"],
        "bw": first["bw"],
        "kp": first["kp"],
        "ki": first["ki"],
        "tau": first.get("tau"),
        "nodes": first["nodes"],
        "n_runs": len(group),
        "follower_node_id": first["follower_node_id"],
        "relayer_node_id": first["relayer_node_id"],
        # Across-runs robust center + spread
        "err_to_gw_median": err_gw_med,
        "err_to_gw_median_iqr": err_gw_iqr,
        "err_to_gw_std": err_gw_std_med,
        "err_between_median": err_bt_med,
        "err_between_median_iqr": err_bt_iqr,
        "err_between_std": err_bt_std_med,
    }


_TN_COL_W = dict(run=4, sf=4, bw=5, kp=4, ki=4, tau=5, err=22)


def _two_node_header():
    cw = _TN_COL_W
    return (
        f"{'#':>{cw['run']}}  "
        f"{'SF':>{cw['sf']}}  "
        f"{'BW':>{cw['bw']}}  "
        f"{'KP':>{cw['kp']}}  "
        f"{'KI':>{cw['ki']}}  "
        f"{'Tau':>{cw['tau']}}  "
        f"{'Err→GW(ms)':>{cw['err']}}  "
        f"{'Err Δ leaf−relay(ms)':>{cw['err']}}  "
    )


def _two_node_row(i, r, aggregate=False):
    cw = _TN_COL_W
    if aggregate:
        gw_iqr = r.get("err_to_gw_median_iqr", 0.0)
        bt_iqr = r.get("err_between_median_iqr", 0.0)
        gw_str = f"${r['err_to_gw_median']:.3f} ±{gw_iqr:.3f}i q r$,"
        bt_str = f"${r['err_between_median']:.3f} ±{bt_iqr:.3f}i q r$,"
    else:
        gw_str = f"${r['err_to_gw_median']:.3f} ±{r['err_to_gw_std']:.3f}$,"
        bt_str = f"${r['err_between_median']:.3f} ±{r['err_between_std']:.3f}$,"
    return (
        f"{i:>{cw['run']}}  "
        f"{str(r['sf']):>{cw['sf']}}  "
        f"{str(r['bw']):>{cw['bw']}}  "
        f"{str(r['kp']):>{cw['kp']}}  "
        f"{str(r['ki']):>{cw['ki']}}  "
        f"{str(r.get('tau', '—')):>{cw['tau']}}  "
        f"{gw_str:>{cw['err']}}  "
        f"{bt_str:>{cw['err']}}  "
    )


def print_two_node_results(results, by=("sf", "bw"), aggregate=True):
    """
    Group 2-node results by `by` and print each configuration as a titled block.

    Columns:
      Err→GW(ms)            — solely-follower's err_ms (median ± spread)
      Err Δ leaf−relay(ms)  — row-aligned (follower − relayer) err_ms

    aggregate=True  — one summary row per group: median ±IQR across runs of
                      each run's median; std column shows the median of
                      per-run stds.
    aggregate=False — one row per run inside each group: per-run median ± std.
    """
    grouped = group_results(results, by=by)
    if not grouped:
        print("No results to print.")
        return
    for key, group in grouped.items():
        label = "  ".join(f"{k.upper()}={v}" for k, v in zip(by, key))
        header = _two_node_header()
        sep = "─" * len(header)
        title = f"{label}  (n={len(group)})" if aggregate else label
        print(f"\n{'── ' + title + ' ':─<{len(header)}}")
        print(header)
        print(sep)
        if aggregate:
            print(_two_node_row(1, _aggregate_two_node_group(group), aggregate=True))
        else:
            for i, r in enumerate(group, start=1):
                print(_two_node_row(i, r, aggregate=False))


def plot_grouped_results(
    results,
    by=("sf", "bw"),
    title="Per-run HW delay by modulation",
    y_label: str = "error [ms]",
    n_cols=2,
    sharey="row",
):
    """
    One subplot per modulation group. Each run is plotted as a point on the
    x-axis with y = mean_hw_delay and error bars showing ± std_hw_delay.
    A shaded band behind the line shows the same ±1σ extent.

    `n_cols` controls the subplot grid width; rows are added as needed.
    """
    grouped = group_results(results, by=by)
    n_groups = len(grouped)
    if n_groups == 0:
        print("No results to plot.")
        return

    n_cols = max(1, min(n_cols, n_groups))
    n_rows = int(np.ceil(n_groups / n_cols))
    fig, axes = plt.subplots(
        n_rows, n_cols, figsize=(10, 4 * n_rows), sharey=sharey, squeeze=False
    )

    for i, (key, group) in enumerate(grouped.items()):
        ax = axes[i // n_cols, i % n_cols]
        letter_prefix = f"({chr(ord('a') + i)})    "
        label = letter_prefix + "  ".join(f"{k.upper()}={v}" for k, v in zip(by, key))
        xs = list(range(1, len(group) + 1))
        means = [r["mean_hw_delay"] for r in group]
        stds = [r["std_hw_delay"] for r in group]

        ax.errorbar(
            xs,
            means,
            yerr=stds,
            fmt="o-",
            capsize=5,
            color="steelblue",
            ecolor="steelblue",
            elinewidth=1,
            linewidth=1.5,
            markersize=6,
        )
        ax.fill_between(
            xs,
            [m - s for m, s in zip(means, stds)],
            [m + s for m, s in zip(means, stds)],
            alpha=0.15,
            color="steelblue",
        )
        ax.axhline(0, color="gray", linewidth=0.8, linestyle="--")
        ax.set_title(label)
        ax.set_xlabel("Run #")
        ax.set_xticks(xs)
        ax.grid(True, linestyle="--", alpha=0.4)
    for row in range(n_rows):
        axes[row, 0].set_ylabel(y_label)

    for i in range(n_groups, n_rows * n_cols):
        axes[i // n_cols, i % n_cols].set_visible(False)

    fig.suptitle(title, fontsize=12)
    plt.tight_layout()
    plt.show()
    return fig


def plot_oscillation_traces(
    results,
    by: tuple[str, str] = ("kp", "ki"),
    value_col: str = "hardware_delay",
    y_label: str = "error [ms]",
    figure_title: str = "",
    skip_initial: int = 3,
    n_cols: int = 2,
    sharey="row",
):
    """
    One subplot per group. Overlays the per-sample time series of `value_col`
    from each run's main_stats CSV, with a median trace + IQR band on top.
    Each panel is annotated with two oscillation metrics computed across runs
    (after dropping the first `skip_initial` samples to ignore startup):
      - RMS of value_col per run  (lower = quieter loop)
      - sign-changes per run      (higher = faster oscillation)

    `n_cols` controls the subplot grid width; rows are added as needed.
    """
    grouped = group_results(results, by=by)
    n_groups = len(grouped)
    if n_groups == 0:
        print("No results to plot.")
        return

    n_cols = max(1, min(n_cols, n_groups))
    n_rows = int(np.ceil(n_groups / n_cols))
    fig, axes = plt.subplots(
        n_rows, n_cols, figsize=(11, 4 * n_rows), sharey=sharey, squeeze=False
    )

    for i, (key, group) in enumerate(grouped.items()):
        ax = axes[i // n_cols, i % n_cols]
        label = "  ".join(f"{k.upper()}={v}" for k, v in zip(by, key))
        ax.text(0.1, 0.97, f"({chr(ord('a') + i)})")

        traces = []
        for r in group:
            try:
                df = pd.read_csv(r["hw_stats"])
            except FileNotFoundError, pd.errors.EmptyDataError:
                continue
            if value_col not in df.columns:
                continue
            # hw_stats may contain rows for multiple nodes in a 2-node run;
            # restrict to this result's node so the trace length matches the
            # actual per-node sample count.
            node_id = r.get("node_id")
            if node_id is not None and "node_id" in df.columns:
                df = df[df["node_id"] == node_id]
            traces.append(df[value_col].to_numpy())

        if not traces:
            ax.set_title(f"{label}  (no traces)")
            continue

        min_len = min(len(t) for t in traces)
        arr = np.vstack([t[:min_len] for t in traces])  # (n_runs, min_len)
        xs = np.arange(min_len)

        for trace in arr:
            ax.plot(xs, trace, color="steelblue", alpha=0.25, linewidth=0.8)

        med = np.median(arr, axis=0)
        q25 = np.percentile(arr, 25, axis=0)
        q75 = np.percentile(arr, 75, axis=0)
        ax.plot(xs, med, color="black", linewidth=1.8, label="median")
        ax.fill_between(xs, q25, q75, color="black", alpha=0.15, label="IQR")
        ax.axhline(0, color="gray", linewidth=0.8, linestyle="--")

        post = arr[:, skip_initial:] if arr.shape[1] > skip_initial else arr
        # rms = np.sqrt(np.mean(post**2, axis=1))
        sc = np.sum(np.diff(np.sign(post), axis=1) != 0, axis=1)
        letter_prefix = f"({chr(ord('a') + i)})"
        ax.set_title(
            f"{letter_prefix}    {label}  (n={arr.shape[0]})\n"
            # f"RMS={rms.mean():.2f}±{rms.std():.2f}\n  "
            f"sign-changes/run={sc.mean():.1f}±{sc.std():.1f}"
        )

        ax.set_xlabel("Sample index in run")
        ax.grid(True, linestyle="--", alpha=0.4)

    for row in range(n_rows):
        axes[row, 0].set_ylabel(y_label)

    for i in range(n_groups, n_rows * n_cols):
        axes[i // n_cols, i % n_cols].set_visible(False)

    fig.suptitle(figure_title, fontsize=12)
    plt.tight_layout()
    plt.show()
    return fig


def plot_delta_up_down(
    results,
    sf_bw_list,
    title=r"Mean $\Delta$-up / -down per run",
    n_cols=2,
    sharey="row",
):
    """
    For each (sf, bw) in `sf_bw_list`, plot the per-run mean of `delta_up_ms`
    and `delta_down_ms` (from each run's main_stats CSV).

    One subplot per (sf, bw); x = run #, two series per panel.
    """
    grouped = group_results(results, by=("sf", "bw"))
    selected = [(k, grouped[k]) for k in sf_bw_list if k in grouped]
    missing = [k for k in sf_bw_list if k not in grouped]
    for k in missing:
        print(f"  no runs for SF={k[0]} BW={k[1]}")

    n_groups = len(selected)
    if n_groups == 0:
        print("No matching results to plot.")
        return

    n_cols = max(1, min(n_cols, n_groups))
    n_rows = int(np.ceil(n_groups / n_cols))
    fig, axes = plt.subplots(
        n_rows, n_cols, figsize=(10, 4 * n_rows), sharey=sharey, squeeze=False
    )

    for i, ((sf, bw), group) in enumerate(selected):
        ax = axes[i // n_cols, i % n_cols]
        letter_prefix = f"({chr(ord('a') + i)})    "

        xs, ups, downs = [], [], []
        for j, r in enumerate(group, start=1):
            try:
                df = pd.read_csv(r["main_stats"])
            except FileNotFoundError, pd.errors.EmptyDataError:
                continue
            if "delta_up_ms" not in df.columns or "delta_down_ms" not in df.columns:
                continue
            node_id = r.get("node_id")
            if node_id is not None and "node_id" in df.columns:
                df = df[df["node_id"] == node_id]
            xs.append(j)
            ups.append(float(df["delta_up_ms"].mean()))
            downs.append(float(df["delta_down_ms"].mean()))

        if not xs:
            ax.set_title(f"{letter_prefix}SF={sf} BW={bw}  (no data)")
            continue

        ax.plot(xs, ups, "o-", color="tab:blue", label="mean Δ-up (ms)")
        ax.plot(xs, downs, "s-", color="tab:red", label="mean Δ-down (ms)")
        ax.axhline(0, color="gray", linewidth=0.8, linestyle="--")
        up_mean, up_sd = float(np.mean(ups)), float(np.std(ups))
        down_mean, down_sd = float(np.mean(downs)), float(np.std(downs))
        ax.set_title(
            f"{letter_prefix}SF={sf} BW={bw}  (n={len(xs)})\n"
            f"Δ-up={up_mean:.3f}±{up_sd:.3f} ms, \n"
            f"Δ-down={down_mean:.3f}±{down_sd:.3f} ms"
        )
        ax.set_xlabel("Run #")
        ax.set_xticks(xs)
        ax.grid(True, linestyle="--", alpha=0.4)
        ax.legend(fontsize=8)

    for row in range(n_rows):
        axes[row, 0].set_ylabel("mean Δ per run (ms)")

    for i in range(n_groups, n_rows * n_cols):
        axes[i // n_cols, i % n_cols].set_visible(False)

    fig.suptitle(title, fontsize=12)
    plt.tight_layout()
    plt.show()
    return fig

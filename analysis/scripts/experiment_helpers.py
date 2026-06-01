from collections import defaultdict

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd


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


# ── Data crunching ────────────────────────────────────────────────────────────


def analyze_runs(runs, min_entries=10):
    """
    Takes a list of run dicts and returns a list of result dicts, one per run.
    Runs whose CSV files are missing, empty, or have fewer than `min_entries`
    data rows are skipped with a printed warning (same strategy as
    load_and_weight_datasets).

      sf, bw, kp, ki, nodes,
      mean_hw_delay, std_hw_delay,
      pct_in_low  (fraction 0–1),
      settling_index  (first row where tau_hb_high is True, or None)
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

        if len(stats) < min_entries or len(hw_stats) < min_entries:
            print(
                f"  skip {label}: too few rows (main={len(stats)}, hw={len(hw_stats)}, need >{min_entries})"
            )
            continue

        mean = hw_stats.mean()
        std = hw_stats.std()
        pct_in_low, first_index = get_pct_in_low(stats)

        results.append(
            {
                "sf": run.get("sf"),
                "bw": run.get("bw"),
                "kp": run.get("kp"),
                "ki": run.get("ki"),
                "nodes": len(run.get("nodes", [])),
                "mean_hw_delay": float(mean.iloc[0]),
                "std_hw_delay": float(std.iloc[0]),
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

    return {
        "sf": first["sf"],
        "bw": first["bw"],
        "kp": first["kp"],
        "ki": first["ki"],
        "nodes": first["nodes"],
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
    }


# ── Printing ──────────────────────────────────────────────────────────────────

_COL_W = dict(
    run=4, sf=4, bw=5, kp=4, ki=4, nodes=5, mean=18, std=14, pct=13, settle=11
)


def _table_header(aggregate=False):
    cw = _COL_W
    center_label = "Median(µs)" if aggregate else "Mean(µs)"
    std_label = "Std(µs)" if aggregate else "Std(µs)"
    pct_label = "%InLow"
    settle_label = "Settle"
    return (
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


def _table_row(i, r, aggregate=False):
    cw = _COL_W
    settle_val = r["settling_index"]

    if aggregate:
        iqr = r.get("mean_hw_delay_iqr", 0.0)
        std_sd = r.get("std_hw_delay_sd", 0.0)
        plow_sd = r.get("pct_in_low_sd", 0.0)
        s_sd = r.get("settling_sd", 0.0)

        # median ±IQR (IQR = Q75-Q25, a robust spread measure)
        mean_str = f"${r['mean_hw_delay']:.1f} ±{iqr:.1f}iqr$,"
        std_str = f"${r['std_hw_delay']:.2f} ±{std_sd:.2f}$,"
        plow_str = f"${r['pct_in_low'] * 100:.1f} ±{plow_sd * 100:.1f}%$,"
        settle_str = f"${settle_val} ±{s_sd:.0f}$," if settle_val is not None else "—"
    else:
        sd_tag = f" $±{r['mean_hw_delay_sd']:.2f}$," if "mean_hw_delay_sd" in r else ""
        mean_str = f"${r['mean_hw_delay']:.3f}{sd_tag}$,"
        std_str = f"${r['std_hw_delay']:.3f}$,"
        plow_str = f"${r['pct_in_low'] * 100:.1f}%$,"
        settle_str = str(settle_val) if settle_val is not None else "—"

    return (
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


def print_run_results(results, title=None, aggregate=False):
    """Pretty-print a flat list of result dicts as a table."""
    header = _table_header(aggregate=aggregate)
    sep = "─" * len(header)
    if title:
        print(f"\n{'── ' + title + ' ':─<{len(header)}}")
    else:
        print(sep)
    print(header)
    print(sep)
    for i, r in enumerate(results, start=1):
        print(_table_row(i, r, aggregate=aggregate))


def print_grouped_results(results, by=("sf", "bw"), aggregate=True):
    """
    Group results by `by`, then print each configuration as a titled block.

    aggregate=True  (default) — one summary row per group:
                    median ±IQR for the per-run mean (robust to outliers),
                    mean ±std across runs for Std, %InLow, and Settle.
    aggregate=False — show every individual run inside each group.
    """
    grouped = group_results(results, by=by)
    for key, group in grouped.items():
        label = "  ".join(f"{k.upper()}={v}" for k, v in zip(by, key))
        if aggregate:
            print_run_results(
                [aggregate_group(group)],
                title=f"{label}  (n={len(group)})",
                aggregate=True,
            )
        else:
            print_run_results(group, title=label)
        # print()


# ── Plotting ──────────────────────────────────────────────────────────────────


def plot_grouped_results(
    results, by=("sf", "bw"), title="Per-run HW delay by modulation"
):
    """
    One subplot per modulation group. Each run is plotted as a point on the
    x-axis with y = mean_hw_delay and error bars showing ± std_hw_delay.
    A shaded band behind the line shows the same ±1σ extent.
    """
    grouped = group_results(results, by=by)
    n_groups = len(grouped)
    if n_groups == 0:
        print("No results to plot.")
        return

    n_cols = 2
    n_rows = int(np.ceil(n_groups / n_cols))
    fig, axes = plt.subplots(
        n_rows, n_cols, figsize=(10, 4 * n_rows), sharey=True, squeeze=False
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
        axes[row, 0].set_ylabel("HW delay mean ± std (µs)")

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
    skip_initial: int = 3,
):
    """
    One subplot per group. Overlays the per-sample time series of `value_col`
    from each run's main_stats CSV, with a median trace + IQR band on top.
    Each panel is annotated with two oscillation metrics computed across runs
    (after dropping the first `skip_initial` samples to ignore startup):
      - RMS of value_col per run  (lower = quieter loop)
      - sign-changes per run      (higher = faster oscillation)
    """
    grouped = group_results(results, by=by)
    n_groups = len(grouped)
    if n_groups == 0:
        print("No results to plot.")
        return
    print(n_groups)

    n_cols = 2
    n_rows = int(np.ceil(n_groups / n_cols))
    fig, axes = plt.subplots(
        n_rows, n_cols, figsize=(11, 4 * n_rows), sharey=True, squeeze=False
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
        rms = np.sqrt(np.mean(post**2, axis=1))
        sc = np.sum(np.diff(np.sign(post), axis=1) != 0, axis=1)
        letter_prefix = f"({chr(ord('a') + i)})"
        ax.set_title(
            f"{letter_prefix}    {label}  (n={arr.shape[0]})\n"
            f"RMS={rms.mean():.2f}±{rms.std():.2f}  "
            f"sign-changes/run={sc.mean():.1f}±{sc.std():.1f}"
        )
        ax.set_xlabel("Sample index in run")
        ax.grid(True, linestyle="--", alpha=0.4)

    for row in range(n_rows):
        axes[row, 0].set_ylabel(value_col)

    for i in range(n_groups, n_rows * n_cols):
        axes[i // n_cols, i % n_cols].set_visible(False)

    fig.suptitle(
        f"Oscillation traces: {value_col} per run, grouped by {by}", fontsize=12
    )
    plt.tight_layout()
    plt.show()
    return fig

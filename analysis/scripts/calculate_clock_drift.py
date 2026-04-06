import polars as pl
import numpy as np
import sys

from polars.io import delta

def calculate_trigger_deltas(csv_filepath: str, ref_channel='DIN 1', target_channel='DIN 0'):

    with open(csv_filepath, 'r') as f:
        lines = f.readlines()
    header_idx = next(i for i, line in enumerate(lines) if line.startswith('Time (s)'))
    
    df = pl.read_csv(csv_filepath, skip_rows=header_idx)
    
    # Clean up column names to prevent trailing/leading spaces
    df = df.rename({col: col.strip() for col in df.columns})
    
    # Clean channel names and filter for Rising Edges (Data == 1)
    edges = df.with_columns(
        pl.col("Channel").str.replace_all(r"\s+", " ").str.strip_chars()
    ).filter(pl.col("Data") == 1)
    
    # Separate target and reference triggers, rename the time columns, and ensure they are sorted
    target_times = edges.filter(pl.col("Channel") == target_channel).select(
        pl.col("Time (s)").alias("Time_Target")
    ).sort("Time_Target")
    
    ref_times = edges.filter(pl.col("Channel") == ref_channel).select(
        pl.col("Time (s)").alias("Time_Ref")
    ).sort("Time_Ref")
    
    print(f"Found {target_times.height} triggers on {target_channel}")
    print(f"Found {ref_times.height} triggers on {ref_channel}\n")
    
    # Automatically pairs each Target trigger with the nearest Reference trigger
    matched = target_times.join_asof(
        ref_times,
        left_on="Time_Target",
        right_on="Time_Ref",
        strategy="nearest"
    )
    
    # Calculate Deltas (in milliseconds) and filter out exact 0.0s (initialization artifact)
    matched = matched.with_columns(
        ((pl.col("Time_Target") - pl.col("Time_Ref")).abs() * 1000).alias("Delta_ms")
    ).filter(pl.col("Delta_ms") != 0.0)
    
    deltas_ms = matched["Delta_ms"].to_numpy()
    
    mean_delta = np.mean(deltas_ms)
    median_delta = np.median(deltas_ms)
    
    print("=== SYNCHRONIZATION METRICS ===")
    print(f"Total Valid Pairs Matched: {len(deltas_ms)}")
    if len(deltas_ms) < 30:
        print("Individual Time-Deltas (ms):")
        for i, d in enumerate(deltas_ms, 1):
            print(f"  Event {i}: {d:.4f} ms")
    
    print(f"\nAverage Delta: {mean_delta:.4f} ms")
    print(f"Median Delta: {median_delta:.4f} ms")
    
    return deltas_ms

if __name__ == "__main__":
    if len(sys.argv) > 1:
        file_path = sys.argv[1]
    else:
        file_path = "../data/fredag1303.csv"
    calculate_trigger_deltas(file_path)

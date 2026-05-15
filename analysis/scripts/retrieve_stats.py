import glob

import numpy as np
import pandas as pd


def load_and_weight_datasets(file_pattern="data/main/*.csv", min_entries=10):
    """
    Loads multiple CSV files, filters out small ones, and assigns weights
    based on observation index.

    Args:
        file_pattern (str): The path pattern to find your CSVs (e.g., 'data/*.csv' or a list of files).
        min_entries (int): Minimum number of rows required to keep the dataset.

    Returns:
        pd.DataFrame: A combined dataframe with a new 'sample_weight' column.
    """
    # If a string is provided, find all matching files. Otherwise, assume it's a list.
    if isinstance(file_pattern, str):
        file_paths = glob.glob(file_pattern)
    else:
        file_paths = file_pattern

    valid_dfs = []

    for file in file_paths:
        try:
            df = pd.read_csv(file)
            # Filter out any big errors
            df = df[df["err_ms"] <= 100]
            df = df[df["delta_up_ms"] <= 100]
            df = df[df["delta_down_ms"] <= 100]
            # Check condition: amount of entries must be greater than min_entries (10)
            if len(df) > min_entries:
                # Weighting strategy: Weight according to the length,
                # root it get a smoother weight curve
                df["sample_weight"] = np.sqrt(len(df))

                valid_dfs.append(df)
            else:
                print(
                    f"Skipping {file}: Only {len(df)} entries (needs > {min_entries})."
                )

        except Exception as e:
            print(f"Error reading {file}: {e}")

    # Combine all valid dataframes into one master dataframe
    if not valid_dfs:
        print("Warning: No valid dataframes found matching the criteria.")
        return pd.DataFrame()

    combined_df = pd.concat(valid_dfs, ignore_index=True)
    return combined_df


#

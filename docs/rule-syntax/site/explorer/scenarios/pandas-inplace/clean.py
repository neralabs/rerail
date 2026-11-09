import pandas as pd


def clean(frame: pd.DataFrame):
    frame.dropna(inplace=True)
    frame.rename(columns={"a": "b"}, inplace=True)
    return frame


def clean_ok(frame: pd.DataFrame):
    return frame.dropna().rename(columns={"a": "b"})

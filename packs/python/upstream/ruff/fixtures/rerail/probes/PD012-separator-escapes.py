import pandas as pd

pd.read_table("a", sep="\N{COMMA}")
pd.read_table("a", sep=",")
pd.read_table("a", sep="\U0000002c")
pd.read_table("a", sep=r"\x2c")
pd.read_table("a", sep=",\
")
pd.read_table("a", sep=", ")
pd.read_table("a", sep=" ,")
pd.read_table("a", sep="\,")

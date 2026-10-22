import pandas as pd

__all__ = x = pd.Series([1])
__all__.nunique() == 1
x.nunique() == 1


def f():
    __all__ = pd.Series([1])
    return __all__.nunique() == 1


with ctx([s.nunique() == 1 for s in frames]):
    pass
[t.nunique() == 1 for t in frames]

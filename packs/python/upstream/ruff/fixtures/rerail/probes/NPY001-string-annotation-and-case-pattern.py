from typing import Optional

import numpy as np


def f(value: "Optional[np.float]") -> None:
    match value:
        case np.int():
            pass

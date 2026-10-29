import sys
if sys.platform > "linux":
    x: int

import sys
if sys.platform > "linux":  # noqa
    x: int

import sys
if sys.platform > "linux":  # noqa: PYI007
    x: int

import sys
if sys.platform > "linux":  # NOQA: PYI007
    x: int

import sys
if sys.platform > "linux":  # noqa: E501
    x: int

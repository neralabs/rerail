import sys
if sys.version_info >= (3, "9"):
    x: int

import sys
if sys.version_info >= (3, "9"):  # noqa
    x: int

import sys
if sys.version_info >= (3, "9"):  # noqa: PYI003
    x: int

import sys
if sys.version_info >= (3, "9"):  # NOQA: PYI003
    x: int

import sys
if sys.version_info >= (3, "9"):  # noqa: E501
    x: int

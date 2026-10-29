import sys
if sys.version_info >= (3, 4, 3):
    x: int

import sys
if sys.version_info >= (3, 4, 3):  # noqa
    x: int

import sys
if sys.version_info >= (3, 4, 3):  # noqa: PYI004
    x: int

import sys
if sys.version_info >= (3, 4, 3):  # NOQA: PYI004
    x: int

import sys
if sys.version_info >= (3, 4, 3):  # noqa: E501
    x: int

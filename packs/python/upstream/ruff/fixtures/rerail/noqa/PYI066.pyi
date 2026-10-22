import sys

if sys.version_info < (3, 9):  
    a: int
else:
    a: str

import sys

if sys.version_info < (3, 9):    # noqa
    a: int
else:
    a: str

import sys

if sys.version_info < (3, 9):    # noqa: PYI066
    a: int
else:
    a: str

import sys

if sys.version_info < (3, 9):    # NOQA: PYI066
    a: int
else:
    a: str

import sys

if sys.version_info < (3, 9):    # noqa: E501
    a: int
else:
    a: str

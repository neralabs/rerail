import sys
if sys.version_info[:2] == (3,):
    x: int

import sys
if sys.version_info[:2] == (3,):  # noqa
    x: int

import sys
if sys.version_info[:2] == (3,):  # noqa: PYI005
    x: int

import sys
if sys.version_info[:2] == (3,):  # NOQA: PYI005
    x: int

import sys
if sys.version_info[:2] == (3,):  # noqa: E501
    x: int

import sys
if sys:
    x: int

import sys
if sys:  # noqa
    x: int

import sys
if sys:  # noqa: PYI002
    x: int

import sys
if sys:  # NOQA: PYI002
    x: int

import sys
if sys:  # noqa: E501
    x: int

import sys
if sys.platform == "linus":
    x: int

import sys
if sys.platform == "linus":  # noqa
    x: int

import sys
if sys.platform == "linus":  # noqa: PYI008
    x: int

import sys
if sys.platform == "linus":  # NOQA: PYI008
    x: int

import sys
if sys.platform == "linus":  # noqa: E501
    x: int

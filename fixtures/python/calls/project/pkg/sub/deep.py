"""Relative imports two dots up: `..` is `pkg`, from `pkg.sub.deep`."""
from .. import nested
from ..util import helper


def exercise():
    helper(10)
    nested.outer(11)

import pkg.util
import pkg.util as u
from pkg import util as util_module
from pkg.util import leaf as renamed_leaf

from . import nested
from .models import Base as B, Child
from .util import *  # noqa: F403

run_helper = u.helper


def exercise():
    pkg.util.helper(1)
    u.helper(2)
    util_module.leaf(3)
    renamed_leaf(4)
    B.make(5)
    Child(6)
    run_helper(7)
    defined_later()  # noqa: F405
    nested.outer(1)

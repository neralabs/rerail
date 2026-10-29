from . import sibling
from . . import parent
from . .pkg import mod as alias, other
from .\
 .backslash import thing
from ... .deep import *
from ..import nospace
from ..import*
from .. import (
    a,
    b as c,
)  # noqa: TID252

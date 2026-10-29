import logging
import typing
from typing import Annotated, NewType, TypeAlias, TypeVar, cast

logger = logging.getLogger(__name__)


def f():
    local: "logger.exception('local annotation')" = 1
    local2: "logger.info('local annotation', exc_info=True)"
    local3: "logger.exception('local annotation', exc_info=False)" = 1


a1: Annotated[int, "logger.exception('annotated metadata')"] = 1
a2: Annotated["logger.exception('annotated type')", 1] = 1
A3: TypeAlias = "logger.exception('typealias value')"
A4: TypeAlias = "logger.info('typealias value', exc_info=True)"
A5: TypeAlias = "logger.exception('typealias value', exc_info=None)"
N = NewType("N", "logger.exception('newtype')")
T = TypeVar("T", "logger.exception('constraint')", "logger.info('c', exc_info=True)")
c1 = cast(typ="logger.exception('cast kw')", val=1)
c2 = typing.cast("logger.exception('cast', exc_info=False)", val=1)
c3 = cast(1, "logger.exception('cast second')")


class K:
    attr: "logger.exception('class annotation')"
    attr2: "logger.info('class annotation', exc_info=True)"


if typing.TYPE_CHECKING:
    tc: "logger.exception('type checking')" = 1


def g(*args: "logger.exception('star')", **kw: "logger.exception('kwargs', exc_info=0)"):
    pass


x = 1  # type: "logger.exception('type comment')"

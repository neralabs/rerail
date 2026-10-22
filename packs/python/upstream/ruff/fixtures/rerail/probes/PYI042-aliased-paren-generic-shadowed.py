from typing import TypeAlias as TA
import typing_extensions as te

snake_aliased: TA = int
(snake_paren): te.TypeAlias = str
snake_in_parens: (TA) = bytes
type snake_pep695[T] = list[T]
type CamelPep695[T] = list[T]


class Holder:
    snake_member: TA = int
    self_attr: TA


def func():
    TA = int
    not_an_alias: TA = int

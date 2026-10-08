from typing import Dict
import typing

ages = {"Tom": 23}
ages.get("Cat", None)
ages = ["Tom"]
ages.get("Cat", None)
comprehension = {k: v for k, v in pairs}
comprehension.get("a", None)
called = dict()
called.get("a", None)
annotated: dict = make()
annotated.get("a", None)
generic: dict[str, int] = make()
generic.get("a", None)
aliased: Dict[str, int] = make()
aliased.get("a", None)
qualified: typing.Dict = make()
qualified.get("a", None)
other: object = make()
other.get("a", None)
first = second = {}
first.get("a", None)
second.get("a", None)
(parenthesized) = {}
parenthesized.get("a", None)
if (walrus := {}):
    walrus.get("a", None)
{}.get("a", None)
unknown.get("a", None)
ages2 = {}
ages2.get("a", "default")
ages2.get("a")


def function(parameter: dict, plain, *args, **kwargs):
    parameter.get("a", None)
    plain.get("a", None)
    kwargs.get("a", None)
    args.get("a", None)


def later():
    return late.get("a", None)


late = {}


class Shadow:
    dict = list
    local: dict = make()
    local.get("a", None)


def shadowed(dict):
    value: dict = make()
    return value.get("a", None)

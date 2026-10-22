from typing import TypedDict

def f():
    a: TypedDict("E", total="int") = 1
    b: TypedDict("E", {"k": int}, foo="int") = 1
    c: TypedDict("E", None, foo="int") = 1
    d: TypedDict("E", extra_items="int") = 1
    e: TypedDict("E", foo="a b") = 1
    g: TypedDict("E", foo="list[") = 1
    i: TypedDict("E", {"k": "int"}) = 1
    j: TypedDict("E", **{"k": "int"}) = 1
    k: TypedDict("E", foo="int | None") = 1

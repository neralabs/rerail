from typing import Optional, cast, TypeVar, Annotated, Literal
(foo.__dict__  # c
 .get("__annotations__"))
(foo
 # c
 .__dict__
 # d
 ["__annotations__"])
f"{foo.__dict__['__annotations__']}"
f"{foo.__dict__.get('__annotations__')!r:>{10}}"
f"{foo.__dict__['__annotations__']}" f"{foo.__dict__.get('__annotations__', {})}"
x: Optional["foo.__dict__['__annotations__']"] = None
y = cast("foo.__dict__.get('__annotations__')", z)
T = TypeVar("T", bound="foo.__dict__['__annotations__']")
w: Annotated[int, "foo.__dict__['__annotations__']"]
v: Literal["foo.__dict__['__annotations__']"]
u: "list[foo.__dict__['__annotations__']]"
def fn(a: "foo.__dict__['__annotations__']", b: 'foo.__dict__.get("__annotations__")') -> "foo.__dict__['__annotations__']": ...
q = "foo.__dict__['__annotations__']"
print("foo.__dict__.get('__annotations__')")
t: """foo.__dict__['__annotations__']"""
s: ("foo.__dict__"
    "['__annotations__']")
r: "foo.__dict__['__annotations__']" "" = 1
class A:
    z: "foo.__dict__['__annotations__']"
    def m(self) -> "foo.__dict__.get('__annotations__')": ...

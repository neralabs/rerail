from typing import TypedDict
A = TypedDict("A", {"a": int}, {"b": int}, total=False)
B = TypedDict("B", {"a": int} if x else {"b": int})
C = TypedDict("C", ({"a": int}))
D = TypedDict("D", (dict(a=int)))

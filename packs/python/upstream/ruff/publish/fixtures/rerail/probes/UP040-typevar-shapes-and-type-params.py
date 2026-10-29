from typing import ParamSpec, TypeAlias, TypeAliasType, TypeVar

T1 = TypeVar(f"T1", default=int)
T2 = TypeVar(b"T2", default=int)
P1 = ParamSpec("P1", **kw)
T = TypeVar("T")
U = TypeVar("U")

a1: TypeAlias = list[T1]
a2: TypeAlias = list[T2]
a3: TypeAlias = Callable[P1, int]
A = TypeAliasType("A", list[T], type_params=((T),))
B = TypeAliasType("B", list[T], type_params=(T, (U)))
X = X = TypeAliasType("X", int)
Y = Z = TypeAliasType("Z", int)

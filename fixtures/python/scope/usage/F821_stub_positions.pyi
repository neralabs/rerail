import builtins
import collections.abc
import typing
import typing as t
from typing import Annotated, Literal, NamedTuple, TypeAlias, TypedDict, TypeVar, cast
from typing_extensions import NewType as NT, TypeAliasType, TypeForm
from mypy_extensions import Arg

# Runtime positions read in order: each name is bound only at the end.
value = Late
print(late_call)
@late_decorator
def decorated(x: int = late_default) -> None: ...
if late_condition: ...
called = late_function(Late)
listed = [Late, {late_key: Late}]
class Kept: ...
kept = Kept
del Kept
after_del = Kept

# Annotations, class bases and type definitions read after the module.
annotated: Late
def signature(x: Late = ...) -> Late: ...
class Based(Late, metaclass=Late, extra=Late): ...
walrus_cast = cast((walrus := Late), late_value)
print(walrus)
walrus_annotation: walrus
Alias: TypeAlias = Late
AliasT: t.TypeAlias = Late
cast_positional = cast(Late, late_value)
cast_keyword = cast(typ=Late, val=late_value)
cast_module = typing.cast(Late, late_value)
V = TypeVar(late_name, Late, bound=Late, default=Late, covariant=late_flag)
N = NT(late_name, Late)
N2 = NT("N2", tp=Late)
TA = TypeAliasType(late_name, Late, type_params=(Late,))
TA2 = TypeAliasType("TA2", value=Late)
Tup = NamedTuple(late_name, [("a", Late), (late_field, Late), late_element])
Tup2 = NamedTuple("Tup2", a=Late)
Tup3 = NamedTuple("Tup3", **{"a": Late, late_key: Late})
Tup4 = NamedTuple("Tup4", **late_mapping)
TD = TypedDict(late_name, {"a": Late, late_key: Late}, total=late_flag, extra_items=Late)
TDLegacy = TypedDict("TDLegacy", legacy=Late)
TDNone = TypedDict("TDNone", None, legacy=Late)
TDUnpacked = TypedDict("TDUnpacked", {}, **Late)
ArgPositional = Arg(Late, late_name)
ArgKeyword = Arg(type=Late, name=late_name)
Form = TypeForm(Late, late_value)
generic_list = list[Late]
generic_typing = typing.Optional[Late]
generic_abc = collections.abc.Callable[[Late], Late]
generic_builtins = builtins.dict[str, Late]
annotated_meta = Annotated[Late, late_metadata]
annotated_single = Annotated[late_single]
literal = Literal[late_literal]
generic_in_call = late_function(list[Late])
nested = late_function(cast(Late, late_value))
hidden = cast(Late, **never_read)
parenthesized = Annotated[(Late, late_metadata)]
inline = TypedDict[{"a": Late, late_key: Late}]
starred = TypeVar("starred", *Late)
container = {}
container[list[late_store]] += 1
container[list[late_store]] = 1
del container[list[late_store]]

late_call = late_decorator = late_default = late_condition = late_function = 1
late_key = late_value = late_name = late_flag = late_field = late_element = late_mapping = 1
late_metadata = late_single = late_literal = late_store = 1
class Late: ...

from typing import List, cast

def cast(typ, val): ...
shadowed_cast = cast(Late, 1)
List = dict
shadowed_generic = List[Late]
list = dict
shadowed_builtin = list[Late]
class Late: ...

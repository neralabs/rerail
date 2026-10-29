from typing import Annotated, List, Optional
a: Optional[Annotated[int, "List[int]"]]
b = Optional[Annotated[int, "List[int]"]]
c: Annotated[("List[int]", "List[int]")]
d = Annotated[("List[int]", "List[int]"), "List[int]"]
e: Optional[Annotated["List[int]", "List[int]"]]

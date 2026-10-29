from dataclasses import dataclass
from typing import Annotated, Union


@dataclass
class A:
    a: Union[(int, str)] = []
    b: Union[((int, str))] = []
    c: Union[(int, list)] = []
    d: Annotated[(int, 1)] = []
    e: Annotated[(list, 1)] = []
    f: frozendict = []

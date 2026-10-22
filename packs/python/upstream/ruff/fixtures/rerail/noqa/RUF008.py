from dataclasses import dataclass

@dataclass
class A:
    x: list[int] = []  

from dataclasses import dataclass

@dataclass
class A:
    x: list[int] = []    # noqa

from dataclasses import dataclass

@dataclass
class A:
    x: list[int] = []    # noqa: RUF008

from dataclasses import dataclass

@dataclass
class A:
    x: list[int] = []    # NOQA: RUF008

from dataclasses import dataclass

@dataclass
class A:
    x: list[int] = []    # noqa: E501

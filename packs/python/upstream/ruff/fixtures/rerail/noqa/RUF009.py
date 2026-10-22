from dataclasses import dataclass

def f() -> list[int]:
    return []

@dataclass
class A:
    x: list[int] = f()  

from dataclasses import dataclass

def f() -> list[int]:
    return []

@dataclass
class A:
    x: list[int] = f()    # noqa

from dataclasses import dataclass

def f() -> list[int]:
    return []

@dataclass
class A:
    x: list[int] = f()    # noqa: RUF009

from dataclasses import dataclass

def f() -> list[int]:
    return []

@dataclass
class A:
    x: list[int] = f()    # NOQA: RUF009

from dataclasses import dataclass

def f() -> list[int]:
    return []

@dataclass
class A:
    x: list[int] = f()    # noqa: E501

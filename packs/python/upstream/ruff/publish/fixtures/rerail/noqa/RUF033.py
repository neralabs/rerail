from dataclasses import dataclass, InitVar
@dataclass
class A:
    a: InitVar[int] = 1
    def __post_init__(self, a: int = 1) -> None:  
        pass

from dataclasses import dataclass, InitVar
@dataclass
class A:
    a: InitVar[int] = 1
    def __post_init__(self, a: int = 1) -> None:    # noqa
        pass

from dataclasses import dataclass, InitVar
@dataclass
class A:
    a: InitVar[int] = 1
    def __post_init__(self, a: int = 1) -> None:    # noqa: RUF033
        pass

from dataclasses import dataclass, InitVar
@dataclass
class A:
    a: InitVar[int] = 1
    def __post_init__(self, a: int = 1) -> None:    # NOQA: RUF033
        pass

from dataclasses import dataclass, InitVar
@dataclass
class A:
    a: InitVar[int] = 1
    def __post_init__(self, a: int = 1) -> None:    # noqa: E501
        pass

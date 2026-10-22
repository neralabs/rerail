from dataclasses import dataclass
from enum import Enum
@dataclass  
class A(Enum):
    X = 1

from dataclasses import dataclass
from enum import Enum
@dataclass    # noqa
class A(Enum):
    X = 1

from dataclasses import dataclass
from enum import Enum
@dataclass    # noqa: RUF049
class A(Enum):
    X = 1

from dataclasses import dataclass
from enum import Enum
@dataclass    # NOQA: RUF049
class A(Enum):
    X = 1

from dataclasses import dataclass
from enum import Enum
@dataclass    # noqa: E501
class A(Enum):
    X = 1

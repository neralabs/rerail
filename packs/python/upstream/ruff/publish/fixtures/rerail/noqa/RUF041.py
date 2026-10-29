from typing import Literal
x: Literal[Literal[1], 2]  

from typing import Literal
x: Literal[Literal[1], 2]    # noqa

from typing import Literal
x: Literal[Literal[1], 2]    # noqa: RUF041

from typing import Literal
x: Literal[Literal[1], 2]    # NOQA: RUF041

from typing import Literal
x: Literal[Literal[1], 2]    # noqa: E501

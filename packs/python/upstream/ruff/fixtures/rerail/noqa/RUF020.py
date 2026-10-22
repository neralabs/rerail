from typing import Never
def f(x: Never | int): ...  

from typing import Never
def f(x: Never | int): ...    # noqa

from typing import Never
def f(x: Never | int): ...    # noqa: RUF020

from typing import Never
def f(x: Never | int): ...    # NOQA: RUF020

from typing import Never
def f(x: Never | int): ...    # noqa: E501

from typing import NoReturn

def f(x: NoReturn): ...  

from typing import NoReturn

def f(x: NoReturn): ...    # noqa

from typing import NoReturn

def f(x: NoReturn): ...    # noqa: PYI050

from typing import NoReturn

def f(x: NoReturn): ...    # NOQA: PYI050

from typing import NoReturn

def f(x: NoReturn): ...    # noqa: E501

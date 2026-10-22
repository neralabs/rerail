from functools import lru_cache

@lru_cache(maxsize=None)
def f():
    pass

from functools import lru_cache

@lru_cache(maxsize=None)  # noqa
def f():
    pass

from functools import lru_cache

@lru_cache(maxsize=None)  # noqa: UP033
def f():
    pass

from functools import lru_cache

@lru_cache(maxsize=None)  # NOQA: UP033
def f():
    pass

from functools import lru_cache

@lru_cache(maxsize=None)  # noqa: E501
def f():
    pass

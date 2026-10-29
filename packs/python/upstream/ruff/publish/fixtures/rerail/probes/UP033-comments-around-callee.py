import functools


@(functools.lru_cache  # c
)(maxsize=None)
def a(): pass


@functools.lru_cache(  # c
    maxsize=None  # d
)
def b(): pass

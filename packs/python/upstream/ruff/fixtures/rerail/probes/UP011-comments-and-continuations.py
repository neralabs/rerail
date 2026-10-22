import functools


@(functools.lru_cache  # a (b)
  )(  # c
  \
)  # d
def f(): ...


@functools.lru_cache(  # )
)
async def g(): ...

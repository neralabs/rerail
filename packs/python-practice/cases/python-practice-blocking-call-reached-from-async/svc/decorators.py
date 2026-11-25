import asyncio
import functools
import time
from concurrent.futures import ThreadPoolExecutor

_pool = ThreadPoolExecutor(4)


def logged(f):
    @functools.wraps(f)
    def wrapper(*args, **kwargs):
        return f(*args, **kwargs)

    return wrapper


def retrying(times):
    def deco(f):
        @functools.wraps(f)
        def wrapper(*args):
            for _ in range(times):
                try:
                    return f(*args)
                except OSError:
                    pass
            return None

        return wrapper

    return deco


def throttled(f):
    @functools.wraps(f)
    def wrapper(*args):
        time.sleep(0.1)
        return f(*args)

    return wrapper


def submitted(f):
    @functools.wraps(f)
    def wrapper(*args):
        return _pool.submit(f, *args)

    return wrapper


def in_thread(f):
    @functools.wraps(f)
    async def wrapper(*args):
        return await asyncio.to_thread(f, *args)

    return wrapper

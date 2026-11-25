import asyncio

import nest_asyncio


def run_sync(coro):
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(coro)
    return asyncio.ensure_future(coro)


def run_sync_var(coro):
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        loop = None
    if loop is None:
        return asyncio.run(coro)
    return loop.create_task(coro)


def run_sync_private(coro):
    loop = asyncio._get_running_loop()
    if loop and loop.is_running():
        return loop.create_task(coro)
    else:
        return asyncio.run(coro)


def run_on_loop(loop, coro):
    if not loop.is_running():
        return loop.run_until_complete(coro)
    return asyncio.run_coroutine_threadsafe(coro, loop)


def run_nested(coro):
    nest_asyncio.apply()
    return asyncio.get_event_loop().run_until_complete(coro)


def run_after_try(coro):
    # The try only probes; the loop start below runs either way.
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        pass
    return asyncio.run(coro)


def run_in_else(coro):
    # The try's else runs when a loop runs.
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return None
    else:
        return asyncio.run(coro)


def run_wrong_branch(loop, coro):
    if loop.is_running():
        return loop.run_until_complete(coro)
    return None


def run_elif(loop, coro):
    if loop.is_running():
        return asyncio.run_coroutine_threadsafe(coro, loop)
    elif coro is not None:
        return loop.run_until_complete(coro)
    return None


def run_wrong_except(coro):
    # A handler of another exception says nothing about the loop.
    try:
        asyncio.get_running_loop()
        coro = prepare(coro)
    except ValueError:
        return asyncio.run(coro)
    return None


def prepare(coro):
    return coro

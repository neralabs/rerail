import asyncio

from svc.services.guards import (run_after_try, run_elif, run_in_else, run_nested, run_on_loop, run_sync,
                                 run_sync_private, run_sync_var, run_wrong_branch, run_wrong_except)


async def ping():
    return 1


async def dual(loop):
    a = run_sync(ping())
    b = run_sync_var(ping())
    c = run_sync_private(ping())
    d = run_on_loop(loop, ping())
    e = run_nested(ping())
    f = run_after_try(ping())  # caught
    g = run_in_else(ping())  # caught
    h = run_wrong_branch(loop, ping())  # caught
    i = run_elif(loop, ping())
    j = run_wrong_except(ping())  # caught
    return a, b, c, d, e, f, g, h, i, j


async def guarded_inline(coro):
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(coro)
    return await coro

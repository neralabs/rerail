import trio
async def f(timeout):
    pass

import trio
async def f(timeout):  # noqa
    pass

import trio
async def f(timeout):  # noqa: ASYNC109
    pass

import trio
async def f(timeout):  # NOQA: ASYNC109
    pass

import trio
async def f(timeout):  # noqa: E501
    pass

import trio
async def f():
    await trio.sleep(100000)

import trio
async def f():
    await trio.sleep(100000)  # noqa

import trio
async def f():
    await trio.sleep(100000)  # noqa: ASYNC116

import trio
async def f():
    await trio.sleep(100000)  # NOQA: ASYNC116

import trio
async def f():
    await trio.sleep(100000)  # noqa: E501

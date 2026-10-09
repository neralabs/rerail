import trio
async def f():
    await trio.sleep(0)

import trio
async def f():
    await trio.sleep(0)  # noqa

import trio
async def f():
    await trio.sleep(0)  # noqa: ASYNC115

import trio
async def f():
    await trio.sleep(0)  # NOQA: ASYNC115

import trio
async def f():
    await trio.sleep(0)  # noqa: E501

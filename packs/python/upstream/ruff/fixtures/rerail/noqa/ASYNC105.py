import trio
async def f():
    trio.sleep(0)

import trio
async def f():
    trio.sleep(0)  # noqa

import trio
async def f():
    trio.sleep(0)  # noqa: ASYNC105

import trio
async def f():
    trio.sleep(0)  # NOQA: ASYNC105

import trio
async def f():
    trio.sleep(0)  # noqa: E501

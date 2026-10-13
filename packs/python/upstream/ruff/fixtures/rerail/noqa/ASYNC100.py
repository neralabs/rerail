import asyncio
async def f():
    async with asyncio.timeout(1):
        pass

import asyncio
async def f():
    async with asyncio.timeout(1):  # noqa
        pass

import asyncio
async def f():
    async with asyncio.timeout(1):  # noqa: ASYNC100
        pass

import asyncio
async def f():
    async with asyncio.timeout(1):  # NOQA: ASYNC100
        pass

import asyncio
async def f():
    async with asyncio.timeout(1):  # noqa: E501
        pass

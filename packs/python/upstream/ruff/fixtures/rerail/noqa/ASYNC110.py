import asyncio
async def f(ready):
    while not ready:
        await asyncio.sleep(1)

import asyncio
async def f(ready):
    while not ready:  # noqa
        await asyncio.sleep(1)

import asyncio
async def f(ready):
    while not ready:  # noqa: ASYNC110
        await asyncio.sleep(1)

import asyncio
async def f(ready):
    while not ready:  # NOQA: ASYNC110
        await asyncio.sleep(1)

import asyncio
async def f(ready):
    while not ready:  # noqa: E501
        await asyncio.sleep(1)

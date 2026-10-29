import asyncio

async def f():
    asyncio.create_task(asyncio.sleep(1))  

import asyncio

async def f():
    asyncio.create_task(asyncio.sleep(1))    # noqa

import asyncio

async def f():
    asyncio.create_task(asyncio.sleep(1))    # noqa: RUF006

import asyncio

async def f():
    asyncio.create_task(asyncio.sleep(1))    # NOQA: RUF006

import asyncio

async def f():
    asyncio.create_task(asyncio.sleep(1))    # noqa: E501

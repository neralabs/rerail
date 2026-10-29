import asyncio


async def f(urls):
    t = asyncio.create_task(asyncio.gather(*[fetch() for _ in urls]))
    await t


async def g(urls):
    t = asyncio.create_task(asyncio.gather(*(fetch(u) for u in urls)))
    await t


async def h(urls):
    asyncio.create_task(asyncio.gather(*[fetch() for _ in urls]))


async def i(urls):
    t = asyncio.create_task(asyncio.gather(*[fetch() for _ in urls]))

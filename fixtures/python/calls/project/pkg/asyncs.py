import asyncio


async def fetch(x):
    return helper_sync(x)


def helper_sync(x):
    return x


async def gather_all():
    await fetch(1)
    return await asyncio.gather(fetch(2), fetch(3))


def exercise():
    return asyncio.run(gather_all())

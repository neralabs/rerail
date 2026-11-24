import asyncio

from asgiref.sync import async_to_sync


async def refresh():
    await asyncio.sleep(0)


def refresh_now():
    asyncio.run(refresh())


def refresh_with_loop():
    loop = asyncio.new_event_loop()
    return loop.run_until_complete(refresh())


def refresh_via_asgiref():
    async_to_sync(refresh)()

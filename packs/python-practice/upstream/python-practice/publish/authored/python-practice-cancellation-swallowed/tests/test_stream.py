import asyncio


async def test_cancel_is_swallowed(conn):
    try:
        await conn.read()
    except asyncio.CancelledError:
        pass

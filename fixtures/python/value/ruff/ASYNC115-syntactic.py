import trio


async def f(kw, args, y):
    await trio.sleep(0)
    await trio.sleep(seconds=0)
    await trio.sleep(**{"seconds": 0})
    await trio.sleep(0, **kw)
    await trio.sleep(*args)
    await trio.sleep(*args, 0)
    await trio.sleep(0, seconds=1)
    await trio.sleep(1)
    await trio.sleep((0))
    await trio.sleep(0x0)
    await trio.sleep(0.0)
    await trio.sleep(-0)
    await trio.sleep(seconds=(0))
    await trio.sleep(**kw)

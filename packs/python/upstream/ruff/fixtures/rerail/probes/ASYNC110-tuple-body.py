import trio
async def f(c):
    while c:
        await trio.sleep(1), 1

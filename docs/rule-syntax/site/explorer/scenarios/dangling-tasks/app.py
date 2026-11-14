import asyncio

background = set()


async def notify(user):
    await asyncio.sleep(0)


async def on_signup(user):
    asyncio.create_task(notify(user))


async def on_login(user):
    task = asyncio.create_task(notify(user))
    background.add(task)
    task.add_done_callback(background.discard)

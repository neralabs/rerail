import asyncio
import atexit
import functools
import signal
import threading
from concurrent.futures import ThreadPoolExecutor
from multiprocessing import Process

from mailer import send_mail as external_send
from tornado.ioloop import IOLoop

from . import notify
from .jobs import JobQueue
from .notify import blocking_send, cleanup, plain, send_mail, stream, sync_send


def kick(user):
    send_mail(user)  # caught
    notify.send_mail(user)  # caught
    sync_send(user)
    blocking_send(user)
    plain(user)
    external_send(user)
    asyncio.run(send_mail(user))
    coro = send_mail(user)
    return coro


async def handler(user):
    send_mail(user)  # caught
    stream()  # caught
    await send_mail(user)
    await (notify.send_mail(user))
    task = asyncio.create_task(send_mail(user))
    await asyncio.gather(send_mail(user), notify.send_mail(user))
    async with asyncio.TaskGroup() as tg:
        tg.create_task(send_mail(user))
    async for item in stream():
        print(item)
    return task


async def returns(user):
    return send_mail(user)


def threads(user):
    threading.Thread(target=send_mail, args=(user,)).start()  # caught
    threading.Thread(target=plain, args=(user,)).start()
    threading.Thread(target=functools.partial(send_mail, user)).start()  # caught
    threading.Timer(5, cleanup).start()  # caught
    Process(target=notify.send_mail, args=(user,)).start()  # caught
    with ThreadPoolExecutor() as ex:
        ex.submit(send_mail, user)  # caught
        ex.submit(plain, user)
    atexit.register(cleanup)  # caught
    atexit.register(plain, user)
    signal.signal(signal.SIGTERM, cleanup)  # caught
    threading.Thread(target=sync_send, args=(user,)).start()
    threading.Thread(target=lambda: asyncio.run(send_mail(user))).start()


async def offload(user):
    loop = asyncio.get_running_loop()
    await loop.run_in_executor(None, send_mail, user)  # caught
    await asyncio.to_thread(send_mail, user)  # caught
    await asyncio.to_thread(plain, user)
    await loop.run_in_executor(None, plain, user)
    JobQueue().submit(send_mail, user)
    loop.call_soon(send_mail, user)  # caught
    asyncio.get_running_loop().call_later(1, cleanup)  # caught
    loop.call_soon(plain, user)
    IOLoop.current().call_later(1, cleanup)


@atexit.register  # caught
async def on_exit():
    await cleanup()


send_mail(None)  # caught

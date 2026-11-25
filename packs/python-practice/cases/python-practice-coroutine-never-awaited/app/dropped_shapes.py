import asyncio
import threading

from . import notify
from .notify import plain, send_mail

# ---- a coroutine bound to a name nothing reads ---------------------------------------------------------------------

_ = send_mail(None)  # caught
PENDING = send_mail(None)


def discard_underscore(user):
    _ = send_mail(user)  # caught


async def discard_unread(user):
    result = notify.send_mail(user)  # caught
    await asyncio.sleep(0)


def kept_and_returned(user):
    coro = send_mail(user)
    return coro


async def kept_and_awaited(user):
    pending = send_mail(user)
    await asyncio.sleep(0)
    await pending


def read_in_a_closure(user):
    coro = send_mail(user)

    def later():
        return asyncio.run(coro)

    return later


def sync_value_unread(user):
    _ = plain(user)


# ---- a coroutine call inside a statement whose value is thrown away ----------------------------------------------


def comprehension(users):
    [send_mail(u) for u in users]  # caught


def set_comprehension(users):
    {send_mail(u) for u in users}  # caught


def generator_statement(users):
    (send_mail(u) for u in users)  # caught


def short_circuit(user, ok):
    ok and send_mail(user)  # caught
    user is None or notify.send_mail(user)  # caught
    ok and (user or send_mail(user))  # caught


def gathered(users):
    return asyncio.run(asyncio.wait([send_mail(u) for u in users]))


async def gathered_async(users):
    await asyncio.gather(*[send_mail(u) for u in users])
    ok = users and send_mail(users[0])
    return await ok


def sync_comprehension(users):
    [plain(u) for u in users]


# ---- a lambda that only makes a coroutine, handed to a sync caller ----------------------------------------------


def lambda_targets(user):
    threading.Thread(target=lambda: send_mail(user)).start()  # caught
    threading.Thread(target=lambda: asyncio.run(send_mail(user))).start()
    threading.Thread(target=lambda: plain(user)).start()


async def lambda_callbacks(user):
    loop = asyncio.get_running_loop()
    loop.call_soon(lambda: notify.send_mail(user))  # caught
    loop.call_soon(lambda: asyncio.ensure_future(send_mail(user)))

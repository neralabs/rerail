import asyncio

from svc.services.http import fetch
from svc.services.loops import refresh_now
from svc.services.shell import flush_disks, git_rev


class Server:
    async def init_async(self):
        # Awaited only from the lifespan: startup code, as is what it awaits.
        await self._prepare()
        self.rev = git_rev()

    async def _prepare(self):
        flush_disks()

    async def shutdown_async(self):
        flush_disks()


async def refresh_models():
    return fetch("http://models.local/api")  # caught


async def poll_forever():
    # Spawned as a task from the lifespan: it runs while requests are served.
    while True:
        fetch("http://poll.local/api")  # caught
        await asyncio.sleep(60)


async def nightly():
    return git_rev()  # caught


async def reload_config():
    flush_disks()  # caught


async def warm_cache():
    return git_rev()


async def load_rev():
    return git_rev()


async def warm_loop_helper():
    # A loop start raises in startup code all the same.
    refresh_now()  # caught

import asyncio
from contextlib import asynccontextmanager

from fastapi import FastAPI

from svc.services.loops import refresh_now
from svc.services.shell import flush_disks, git_rev


@asynccontextmanager
async def lifespan(app):
    # Runs before the server takes requests, and after it stops.
    app.state.rev = git_rev()
    yield
    flush_disks()


app = FastAPI(lifespan=lifespan)


@app.on_event("startup")
async def warm():
    app.state.rev = git_rev()


@app.on_event("startup")
async def warm_loop():
    refresh_now()  # caught


@asynccontextmanager
async def session_scope():
    # An asynccontextmanager that is not a lifespan runs per request.
    rev = git_rev()  # caught
    yield rev


async def handler():
    async with session_scope() as rev:
        await asyncio.sleep(0)
        return rev

import asyncio
from contextlib import asynccontextmanager

from fastapi import FastAPI
from starlette.applications import Starlette

from svc.server import (Server, load_rev, nightly, poll_forever, refresh_models, reload_config, warm_cache,
                        warm_loop_helper)
from svc.services.shell import git_rev

server = Server()


@asynccontextmanager
async def lifespan_app(app):
    # Startup: init_async, refresh_models, nightly and reload_config are awaited here.
    await server.init_async()
    await refresh_models()
    await nightly()
    await reload_config()
    await warm_loop_helper()
    asyncio.create_task(poll_forever())
    yield
    await server.shutdown_async()


app = FastAPI(lifespan=lifespan_app)
app.add_event_handler("startup", warm_cache)
other = Starlette(on_startup=[load_rev])


@app.get("/models")
async def models():
    # refresh_models runs per request too, so its blocking call is reported in svc/server.py.
    return await refresh_models()


@app.post("/admin")
async def admin(handle):
    # An open call by name: reload_config may run here, per request.
    await handle.reload_config()
    return git_rev()  # caught


def schedule(scheduler):
    # nightly is handed to a scheduler: it runs while serving.
    scheduler.add_job(nightly, "cron", hour=3)

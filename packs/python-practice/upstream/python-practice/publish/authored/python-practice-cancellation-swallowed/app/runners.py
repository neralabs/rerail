import asyncio
import functools
import logging
from asyncio import Runner

import anyio
import invoke
import trio
import uvloop

logger = logging.getLogger(__name__)


async def serve_forever(server):
    try:
        await server.serve()
    except asyncio.CancelledError:
        logger.info("bye")
    await server.close()


async def serve_with_runner(server):
    try:
        await server.serve()
    except asyncio.CancelledError:
        logger.info("bye")
    await server.close()


async def serve_in_context(server):
    try:
        await server.serve()
    except asyncio.CancelledError:
        logger.info("bye")
    await server.close()


async def serve_bound(server):
    try:
        await server.serve()
    except asyncio.CancelledError:
        logger.info("bye")
    await server.close()


async def serve_anyio(server):
    try:
        await server.serve()
    except asyncio.CancelledError:
        logger.info("bye")
    await server.close()


async def serve_trio(server, port):
    try:
        await server.serve(port)
    except asyncio.CancelledError:
        logger.info("bye")
    await server.close()


async def deploy(server):
    # invoke.run runs a shell command: it is no event loop, and deploy is no runner's main.
    try:
        await server.deploy()
    except asyncio.CancelledError:  # caught
        logger.info("deploy cancelled")
    await server.close()


async def work(server):
    try:
        await server.work()
    except asyncio.CancelledError:  # caught
        logger.info("work cancelled")
    await server.close()


class Pool:
    def run(self, job):
        return job


def main(server):
    uvloop.run(serve_forever(server))
    Runner().run(serve_with_runner(server))
    with asyncio.Runner(debug=True) as runner:
        runner.run(serve_in_context(server))
    bound = asyncio.Runner()
    bound.run(serve_bound(server))
    anyio.run(serve_anyio, server, backend="asyncio")
    trio.run(functools.partial(serve_trio, server, 8080))
    invoke.run(deploy(server))
    pool = Pool()
    pool.run(work(server))

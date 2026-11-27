from concurrent.futures import CancelledError as PoolCancelled


class CancelledError(Exception):
    """A job the user cancelled: an application error, not asyncio's."""


async def run_job(job):
    try:
        await job.run()
    except CancelledError:
        job.mark("cancelled")


async def run_in_pool(loop, pool, fn):
    try:
        return await loop.run_in_executor(pool, fn)
    except PoolCancelled:
        return None

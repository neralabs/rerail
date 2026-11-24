import asyncio
import functools
import logging

logger = logging.getLogger(__name__)
_tasks = set()


def traced(func):
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        logger.debug("calling %s", func.__name__)
        return func(*args, **kwargs)

    return wrapper


@traced
def safe_create_task(coro, label="background task"):
    """Spawn coro as a task that logs its failures and is kept referenced."""

    async def wrapper():
        try:
            await coro
        except Exception:
            logger.exception("%s failed", label)

    task = asyncio.create_task(wrapper())
    _tasks.add(task)
    task.add_done_callback(_tasks.discard)
    return task

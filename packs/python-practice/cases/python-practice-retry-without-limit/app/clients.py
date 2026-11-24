import tenacity
from tenacity import (
    AsyncRetrying,
    Retrying,
    retry,
    retry_if_exception_type,
    stop_after_attempt,
    stop_after_delay,
    wait_exponential,
)
from tenacity import retry as retrying


@retry  # caught
def fetch_bare(client):
    return client.get("/items")


@retry(wait=wait_exponential(multiplier=1, max=10))  # caught
def fetch_with_wait(client):
    return client.get("/items")


@tenacity.retry(retry=retry_if_exception_type(IOError), reraise=True)  # caught
def fetch_transient_only(client):
    return client.get("/items")


@retrying()  # caught
def fetch_alias(client):
    return client.get("/items")


@tenacity.retry  # caught
def fetch_module_bare(client):
    return client.get("/items")


def fetch_loop(client):
    for attempt in Retrying(wait=wait_exponential(max=5)):  # caught
        with attempt:
            return client.get("/items")


async def fetch_async(client):
    async for attempt in AsyncRetrying(reraise=True):  # caught
        with attempt:
            return await client.get("/items")


RETRYER = tenacity.Retrying(  # caught
    wait=tenacity.wait_fixed(2),
)


def fetch_wrapped(client):
    return retry(wait=wait_exponential())(client.get)("/items")  # caught


# Bounded: stop is given.
@retry(stop=stop_after_attempt(3))
def fetch_three(client):
    return client.get("/items")


@retry(stop=stop_after_delay(10), wait=wait_exponential(max=2), retry=retry_if_exception_type(IOError))
def fetch_ten_seconds(client):
    return client.get("/items")


STOP = stop_after_attempt(5) | stop_after_delay(30)


@tenacity.retry(stop=STOP)
def fetch_shared_stop(client):
    return client.get("/items")


RETRY_KWARGS = {"stop": stop_after_attempt(3), "reraise": True}


@retry(**RETRY_KWARGS)
def fetch_shared_kwargs(client):
    return client.get("/items")


def fetch_loop_bounded(client):
    for attempt in Retrying(stop=stop_after_attempt(4)):
        with attempt:
            return client.get("/items")


async def fetch_async_bounded(client):
    async for attempt in AsyncRetrying(stop=stop_after_delay(5)):
        with attempt:
            return await client.get("/items")

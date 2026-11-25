import retrying
from retry import retry as retry_pkg
from retry.api import retry_call
from retrying import Retrying
from retrying import retry as retrying_retry
from tenacity import retry, retry_if_result, wait_fixed


# ---- retrying: never stops by default -----------------------------------------------------------------------------


@retrying.retry  # caught
def legacy_bare(connect):
    return connect()


@retrying.retry(wait_fixed=1000)  # caught
def legacy(connect):
    return connect()


@retrying_retry(retry_on_exception=lambda exc: isinstance(exc, IOError))  # caught
def legacy_alias(connect):
    return connect()


LEGACY = Retrying(wait_exponential_multiplier=100)  # caught


@retrying.retry(stop_max_attempt_number=5)
def legacy_bounded(connect):
    return connect()


@retrying.retry(stop_max_delay=30_000, wait_fixed=1000)
def legacy_deadline(connect):
    return connect()


@retrying.retry(stop_func=lambda attempts, delay: attempts > 3)
def legacy_stop_func(connect):
    return connect()


# ---- retry: tries=-1 by default ---------------------------------------------------------------------------------


@retry_pkg  # caught
def plain_bare(connect):
    return connect()


@retry_pkg(OSError, delay=1, backoff=2)  # caught
def plain(connect):
    return connect()


def plain_call(connect):
    return retry_call(connect, exceptions=OSError, delay=1)  # caught


@retry_pkg(OSError, tries=5, delay=1)
def plain_bounded(connect):
    return connect()


@retry_pkg(OSError, 3)
def plain_positional(connect):
    return connect()


@retry_pkg(OSError, tries=-1, delay=5)
def plain_forever_on_purpose(connect):
    return connect()


def plain_call_bounded(connect):
    return retry_call(connect, exceptions=OSError, tries=3)


# ---- tenacity: a result predicate is not a stop -----------------------------------------------------------------


@retry(wait=wait_fixed(1), retry=retry_if_result(lambda r: r is None))  # caught
def poll_ready(job):
    return job.status()

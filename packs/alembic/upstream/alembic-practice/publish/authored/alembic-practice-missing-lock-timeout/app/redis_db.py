import logging

from sqlalchemy import create_engine

log = logging.getLogger(__name__)

engine = create_engine("postgresql://app@db/app", connect_args={"options": "-c statement_timeout=60000"})
REDIS_LOCK = {"lock_timeout": 30, "blocking_timeout": 5}


def warn():
    log.warning("lock_timeout is not set for %s", engine.url)

from functools import lru_cache

from sqlalchemy import create_engine

_engine = create_engine("postgresql+psycopg://app@db/app", isolation_level="AUTOCOMMIT")


def get_autocommit_engine():
    return create_engine("postgresql+psycopg://app@db/app", isolation_level="AUTOCOMMIT")


@lru_cache
def get_cached_autocommit_engine():
    engine = create_engine("postgresql+psycopg://app@db/app")
    engine = engine.execution_options(isolation_level="AUTOCOMMIT")
    return engine


def get_shared_engine():
    return _engine


def get_engine():
    return create_engine("postgresql+psycopg://app@db/app")


def get_engine_through(factory=get_engine):
    return get_engine()

from sqlalchemy import insert, text

from app import factories
from app.db import engine
from app.factories import get_autocommit_engine, get_cached_autocommit_engine, get_engine, get_shared_engine
from app.models import users


def add_user_autocommit(name: str) -> None:
    # the factory's engine autocommits
    with get_autocommit_engine().connect() as conn:
        conn.execute(insert(users).values(name=name))


def add_user_cached_autocommit(name: str) -> None:
    eng = get_cached_autocommit_engine()
    with eng.connect() as conn:
        conn.execute(insert(users).values(name=name))


def add_user_shared(name: str) -> None:
    with factories.get_shared_engine().connect() as conn:
        conn.execute(insert(users).values(name=name))
    with get_shared_engine().connect() as conn:
        conn.execute(insert(users).values(name=name))


def add_user_factory(name: str) -> None:
    # this factory's engine is transactional
    with get_engine().connect() as conn:
        conn.execute(insert(users).values(name=name))  # caught


def add_user_two_hops(name: str) -> None:
    with factories.get_engine_through().connect() as conn:
        conn.execute(insert(users).values(name=name))  # caught


def archive_users() -> None:
    with engine.connect() as conn:
        conn.execute(text("WITH old AS (SELECT id FROM users WHERE id < 10) DELETE FROM users WHERE id IN (SELECT id FROM old)"))  # caught


def purge_returning() -> None:
    with engine.connect() as conn:
        conn.execute(  # caught
            text(
                """
                WITH gone AS (DELETE FROM users WHERE name IS NULL RETURNING id)
                SELECT count(*) FROM gone
                """
            )
        )


def recursive_read() -> int:
    with engine.connect() as conn:
        return conn.execute(
            text("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5) SELECT sum(i) FROM n")
        ).scalar_one()


def cte_read_with_update_column() -> list:
    with engine.connect() as conn:
        return conn.execute(text("WITH recent AS (SELECT id, updated_at FROM users) SELECT id FROM recent")).all()


# ---- gaps: left alone, though some of these lose writes ---------------------------------------------------------


def run_write(sql: str) -> None:
    # the SQL is a parameter: the rule cannot tell a write from a read
    with engine.connect() as conn:
        conn.execute(text(sql))


def add_user_maybe(name: str, dry: bool) -> None:
    # any later commit counts, even on one branch only
    with engine.connect() as conn:
        conn.execute(insert(users).values(name=name))
        if not dry:
            conn.commit()


class Writer:
    # a connection kept on the object is not followed into other methods
    def __init__(self) -> None:
        self.conn = engine.connect()

    def write(self, name: str) -> None:
        self.conn.execute(insert(users).values(name=name))

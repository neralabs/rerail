import sqlite3

from sqlalchemy import Connection, create_engine, delete, func, insert, select, text
from sqlalchemy.orm import Session

from app.db import autocommit_engine, engine, maintenance_engine
from app.models import Event, metadata, users


def read_users() -> list:
    with engine.connect() as conn:
        rows = conn.execute(select(users)).all()
        conn.execute(text("SELECT count(*) FROM users"))
        conn.exec_driver_sql("SELECT 1")
        return rows


def add_user_begin(name: str) -> None:
    with engine.begin() as conn:
        conn.execute(insert(users).values(name=name))


def add_user_commit(name: str) -> None:
    with engine.connect() as conn:
        conn.execute(insert(users).values(name=name))
        conn.commit()


def add_user_same_with(name: str) -> None:
    with engine.connect() as conn, conn.begin():
        conn.execute(insert(users).values(name=name))


def add_user_inner_begin(name: str) -> None:
    with engine.connect() as conn:
        with conn.begin():
            conn.execute(insert(users).values(name=name))


def add_user_explicit_tx(name: str) -> None:
    conn = engine.connect()
    trans = conn.begin()
    try:
        conn.execute(insert(users).values(name=name))
        trans.commit()
    except Exception:
        trans.rollback()
        raise
    finally:
        conn.close()


def add_user_savepoint(name: str) -> None:
    with engine.connect() as conn:
        with conn.begin_nested():
            conn.execute(insert(users).values(name=name))


def add_user_commit_in_branch(name: str, dry_run: bool) -> None:
    # A commit after the write in any branch counts: precision first.
    with engine.connect() as conn:
        conn.execute(insert(users).values(name=name))
        if dry_run:
            conn.rollback()
        else:
            conn.commit()


def vacuum_autocommit_conn() -> None:
    with engine.connect() as conn:
        conn = conn.execution_options(isolation_level="AUTOCOMMIT")
        conn.execute(text("DROP TABLE IF EXISTS scratch"))


def vacuum_autocommit_chain() -> None:
    with engine.connect().execution_options(isolation_level="AUTOCOMMIT") as conn:
        conn.execute(text("CREATE INDEX CONCURRENTLY ix_users_name ON users (name)"))


def autocommit_engines() -> None:
    with autocommit_engine.connect() as conn:
        conn.execute(text("CREATE DATABASE tenant_1"))
    with maintenance_engine.connect() as conn:
        conn.execute(delete(Event))


def local_autocommit_engine(url: str) -> None:
    eng = create_engine(url, isolation_level="AUTOCOMMIT")
    with eng.connect() as conn:
        conn.execute(text("DROP DATABASE IF EXISTS scratch"))


def sqlite_ddl() -> None:
    # pysqlite runs DDL outside a transaction: the table is created without a commit.
    eng = create_engine("sqlite:///local.db")
    with eng.connect() as conn:
        conn.execute(text("CREATE VIRTUAL TABLE docs USING fts5(body)"))


def insert_then_count(name: str) -> int:
    # `query` is rebound to a select before the connect() block: only the read runs there.
    query = insert(users).values(name=name)
    with engine.begin() as conn:
        conn.execute(query)
    query = select(func.count()).select_from(users)
    with engine.connect() as conn:
        return conn.execute(query).scalar()


def caller_owns_it(conn: Connection, name: str) -> None:
    conn.execute(insert(users).values(name=name))


def session_write(session: Session, name: str) -> None:
    session.execute(insert(users).values(name=name))


def sqlite3_write(path: str) -> None:
    db = sqlite3.connect(path)
    db.execute("INSERT INTO users (name) VALUES ('x')")


def other_connection(conn: Connection, name: str) -> None:
    with engine.connect() as mine:
        conn.execute(insert(users).values(name=name))
        mine.execute(select(users))


class AutocommitStore:
    def __init__(self, url: str) -> None:
        self.engine = create_engine(url, isolation_level="AUTOCOMMIT")

    def record(self, kind: str) -> None:
        with self.engine.connect() as conn:
            conn.execute(insert(Event).values(kind=kind))


class SqliteSearchIndex:
    def __init__(self) -> None:
        self.engine = create_engine(url="sqlite:///books.db")

    def create_index(self) -> None:
        with self.engine.connect() as connection:
            connection.execute(text("CREATE VIRTUAL TABLE books_fts USING fts5(title)"))
            metadata.create_all(bind=connection)

    def add_row(self, title: str) -> None:
        # DML on SQLite does begin a transaction: this insert is rolled back on close.
        with self.engine.connect() as connection:
            connection.execute(text("INSERT INTO books_fts (title) VALUES (:t)"), {"t": title})  # caught

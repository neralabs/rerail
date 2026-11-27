from sqlalchemy import create_engine, delete, insert, text
from sqlalchemy.dialects.postgresql import insert as pg_insert

from app.db import engine
from app.models import Event, metadata, users
from app.repo import store_and_commit, store_users

PURGE_SQL = "DELETE FROM sessions WHERE expires_at < now()"


def purge_sessions() -> None:
    with engine.connect() as conn:
        conn.execute(text(PURGE_SQL))  # caught


def add_user(name: str) -> None:
    with engine.connect() as conn:
        conn.execute(insert(users).values(name=name))  # caught


def rename_user(user_id: int, name: str) -> None:
    with engine.connect() as conn:
        conn.execute(users.update().where(users.c.id == user_id).values(name=name))  # caught


def drop_user(user_id: int) -> None:
    conn = engine.connect()
    conn.execute(text("DELETE FROM users WHERE id = :id"), {"id": user_id})  # caught
    conn.close()


def purge_events(kind: str) -> None:
    stmt = delete(Event).where(Event.kind == kind)
    with engine.connect() as conn:
        conn.execute(stmt)  # caught


def truncate_audit() -> None:
    with engine.connect() as conn:
        conn.exec_driver_sql("TRUNCATE TABLE audit_log")  # caught


def add_column(table: str) -> None:
    with engine.connect() as conn:
        conn.execute(text(f"ALTER TABLE {table} ADD COLUMN note text"))  # caught


def upsert_user(user_id: int, name: str) -> None:
    with engine.connect() as conn:
        conn.execute(  # caught
            pg_insert(users)
            .values(id=user_id, name=name)
            .on_conflict_do_update(index_elements=[users.c.id], set_={"name": name})
        )


def deactivate_all() -> None:
    sql = """
        UPDATE users SET name = name || ' (inactive)'
    """
    with engine.connect() as conn:
        conn.execute(text(sql))  # caught


def import_batches(batches: list[list[str]]) -> None:
    with engine.connect() as conn:
        for batch in batches:
            if batch:
                conn.execute(insert(users), [{"name": n} for n in batch])  # caught


def commit_then_write(name: str) -> None:
    # The first insert is committed; the second one, after the commit, is rolled back on close.
    with engine.connect() as conn:
        conn.execute(insert(users).values(name=name))
        conn.commit()
        conn.execute(delete(users).where(users.c.name == ""))  # caught


def grant_reader(role: str) -> None:
    eng = create_engine("postgresql+psycopg://admin@db/app")
    with eng.connect() as conn:
        conn.execute(text("GRANT SELECT ON ALL TABLES IN SCHEMA public TO %s" % role))  # caught


def import_through_helper(names: list[str]) -> None:
    # The helper writes on the connection it is handed and leaves the commit to this caller.
    with engine.connect() as conn:
        store_users(conn, names)  # caught


def import_through_committing_helper(names: list[str]) -> None:
    with engine.connect() as conn:
        store_and_commit(conn, names)


def create_schema() -> None:
    with engine.connect() as conn:
        metadata.create_all(conn)  # caught


def reset_schema() -> None:
    with engine.connect() as conn:
        metadata.drop_all(bind=conn)
        metadata.create_all(bind=conn)
        conn.commit()


class EventStore:
    def __init__(self, url: str) -> None:
        self.engine = create_engine(url)

    def record(self, kind: str) -> None:
        with self.engine.connect() as conn:
            conn.execute(insert(Event).values(kind=kind))  # caught

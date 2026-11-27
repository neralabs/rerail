from sqlalchemy import Connection, insert, select

from app.models import users


# The caller owns the transaction: a connection handed in is never reported here.
def store_users(conn: Connection, names: list[str]) -> None:
    conn.execute(insert(users), [{"name": n} for n in names])


def store_and_commit(conn: Connection, names: list[str]) -> None:
    conn.execute(insert(users), [{"name": n} for n in names])
    conn.commit()


def count_users(conn: Connection) -> int:
    return conn.scalar(select(users.c.id).limit(1)) or 0

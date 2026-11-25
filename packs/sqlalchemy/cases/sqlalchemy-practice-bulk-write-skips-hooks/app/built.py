import sqlite3

from sqlalchemy.orm import Session

from app import builders
from app.builders import build_query, build_relabel, build_relabel_named, build_retitle


def relabel_built(session: Session, new):
    session.execute(build_relabel(new))  # caught


def relabel_built_named(session: Session, new):
    session.execute(build_relabel_named(new))  # caught


def relabel_through_module(session: Session, new):
    stmt = builders.build_relabel(new)
    session.execute(stmt)  # caught


def retitle_built(session: Session, new):
    session.execute(build_retitle(new))


def read_built(session: Session):
    return session.execute(build_query()).scalars().all()


def relabel_raw(path, new):
    with sqlite3.connect(path) as conn:
        conn.execute(builders.build_sql(new))

import logging

from fastapi import Depends
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from app.db import SessionLocal
from app.models import Item

log = logging.getLogger(__name__)


def get_db():
    with SessionLocal() as s:
        yield s


# `db` has no annotation and no session-like name: its callers pass it a session.
def import_items(db, names):
    for name in names:
        try:
            db.add(Item(name=name))
            db.commit()
        except IntegrityError:  # caught
            log.warning("duplicate %s", name)


def import_route(names: list[str], db: Session = Depends(get_db)):
    import_items(db, names)


def import_job(names):
    with SessionLocal() as session:
        import_items(db=session, names=names)


class Importer:
    def load(self, conn, names):
        for name in names:
            try:
                conn.add(Item(name=name))
                conn.commit()
            except IntegrityError:  # caught
                log.warning("duplicate %s", name)


def run_importer(names):
    session = SessionLocal()
    Importer().load(session, names)


# ---- left alone -------------------------------------------------------------------------------------------------


class Outbox:
    def add(self, item):
        pass

    def commit(self):
        pass


def flush_outbox(box, names):
    # its only caller passes an Outbox, not a session
    for name in names:
        try:
            box.add(name)
            box.commit()
        except IntegrityError:
            log.warning("duplicate %s", name)


def run_outbox(names):
    flush_outbox(Outbox(), names)


def import_unknown(store, names):
    # no caller in the run: nothing says store is a session
    for name in names:
        try:
            store.add(Item(name=name))
            store.commit()
        except IntegrityError:
            log.warning("duplicate %s", name)

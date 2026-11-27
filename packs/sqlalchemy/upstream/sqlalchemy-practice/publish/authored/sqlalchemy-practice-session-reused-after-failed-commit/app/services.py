import logging
from contextlib import asynccontextmanager

import sqlalchemy.exc as sa_exc
from django.db import IntegrityError as DjangoIntegrityError
from sqlalchemy import exc, func, select, update
from sqlalchemy.exc import DBAPIError, IntegrityError, OperationalError, SQLAlchemyError
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session

from app.db import AsyncSessionLocal, ScopedSession, SessionLocal
from app.models import Item

log = logging.getLogger(__name__)


# ---- 1. the try sits in a loop, and the session outlives the loop body ----------------------------------------------


def import_names(names):
    with SessionLocal() as session:
        for name in names:
            try:
                session.add(Item(name=name))
                session.commit()
            except IntegrityError:  # caught
                log.warning("duplicate %s", name)
                continue


def drain(queue):
    session = SessionLocal()
    while queue:
        item = queue.pop()
        try:
            session.add(item)
            session.flush()
        except (OperationalError, ValueError) as e:  # caught
            if is_fatal(e):
                raise
    session.close()


async def import_async(names):
    async with AsyncSessionLocal() as db:
        for name in names:
            try:
                db.add(Item(name=name))
                await db.commit()
            except Exception as e:  # caught
                log.exception(e)


# ---- 2. code after the try uses the session again -------------------------------------------------------------------


def create_then_count(name):
    session = SessionLocal()
    try:
        session.add(Item(name=name))
        session.flush()
    except sa_exc.IntegrityError:  # caught
        log.warning("exists")
    total = session.scalar(select(func.count(Item.id)))
    session.close()
    return total


def first_ok(items):
    session = SessionLocal()
    for it in items:
        try:
            session.add(it)
            session.flush()
        except DBAPIError:  # caught
            break
    session.commit()


# ---- 3. the session outlives the function: a caller reuses it (app/jobs.py), or it is shared ---------------------


async def rename(session: AsyncSession, item, name):
    item.name = name
    try:
        await session.commit()
    except SQLAlchemyError:  # caught
        return False
    return True


def archive(db_session, item):
    item.archived = True
    try:
        db_session.commit()
    except:  # caught
        log.error("archive failed")


class ItemRepository:
    # not reported: the owner's callers are not followed, so a self.<session> is not known to be reused
    def __init__(self, session: Session):
        self.session = session

    def save(self, item):
        try:
            self.session.add(item)
            self.session.commit()
        except exc.IntegrityError:
            return None
        return item


def touch_all():
    try:
        ScopedSession.commit()
    except BaseException:  # caught
        pass


# ---- 4. the handler itself uses the session -------------------------------------------------------------------------


def finish(item_id):
    with SessionLocal() as session:
        try:
            item = session.get(Item, item_id)
            item.name = "done"
            session.commit()
        except Exception:  # caught
            session.execute(update(Item).where(Item.id == item_id).values(name="failed"))
            session.commit()


# ---- near misses: these stay silent ---------------------------------------------------------------------------------


def import_with_rollback(names):
    with SessionLocal() as session:
        for name in names:
            try:
                session.add(Item(name=name))
                session.commit()
            except IntegrityError:
                session.rollback()


def rename_or_fail(session: Session, item, name):
    item.name = name
    try:
        session.commit()
    except IntegrityError as e:
        raise ValueError("name taken") from e


def rename_branches(session: Session, item, name):
    item.name = name
    try:
        session.commit()
    except IntegrityError as e:
        if "unique" in str(e):
            raise ValueError("name taken") from e
        else:
            raise


def import_fresh_sessions(names):
    # each try opens and closes its own session: the close rolls back before the handler runs
    for name in names:
        try:
            with SessionLocal() as session:
                session.add(Item(name=name))
                session.commit()
        except Exception:
            log.exception("skip %s", name)


def import_session_per_iteration(names):
    for name in names:
        session = SessionLocal()
        try:
            session.add(Item(name=name))
            session.commit()
        except IntegrityError:
            log.warning("duplicate %s", name)
        session.close()


def add_once(session: Session, item):
    # the savepoint rolls back when the error leaves the begin_nested block
    try:
        with session.begin_nested():
            session.add(item)
            session.flush()
    except IntegrityError:
        pass
    session.commit()


def local_and_done(name):
    session = SessionLocal()
    try:
        session.add(Item(name=name))
        session.commit()
    except IntegrityError:
        return None
    finally:
        session.close()


def loop_but_returns(names):
    session = SessionLocal()
    for name in names:
        try:
            session.add(Item(name=name))
            session.commit()
        except IntegrityError:
            return False
    return True


def unrelated_error(session: Session, item):
    try:
        session.commit()
    except ValueError:
        pass


def django_error(session: Session, item):
    try:
        session.commit()
    except DjangoIntegrityError:
        pass


def outer_handler(session: Session, rows):
    # the commit's own try rolls back; the outer handler sees only errors raised outside it
    try:
        for row in rows:
            try:
                session.add(row)
                session.commit()
            except Exception:
                session.rollback()
    except Exception:
        log.exception("batch failed")


@asynccontextmanager
async def get_db_context(db=None):
    if db is not None:
        yield db
    else:
        async with SessionLocal() as session:
            yield session


async def update_name(item_id, name, db: AsyncSession | None = None):
    # open-webui's shape: `db` is rebound by a helper that opens a fresh session by default and closes it on exit
    async with get_db_context(db) as db:
        try:
            item = await db.get(Item, item_id)
            item.name = name
            await db.commit()
            return item
        except Exception:
            return None


class TagService:
    def __init__(self, session: Session):
        self._session = session

    def add(self, item):
        try:
            self._session.add(item)
            self._session.flush()
        except IntegrityError as e:
            # the helper always raises, so the error still leaves the method
            self._conflict(e, item.name)

    def _conflict(self, error, name):
        if "unique" in str(error.orig):
            raise ValueError(f"{name} already exists")
        raise error


def git_commit(repo, message):
    try:
        repo.commit(message)
    except Exception:
        pass


def is_fatal(e):
    return isinstance(e, OperationalError)

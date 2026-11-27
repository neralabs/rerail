from sqlalchemy.orm import Session

from app.db import AsyncSessionLocal, SessionLocal
from app.services import archive, rename


async def rename_all(items, name):
    async with AsyncSessionLocal() as session:
        for item in items:
            # rename swallows a failed commit: the next iteration passes it the same session
            await rename(session, item, name)


def archive_and_count(item):
    session = SessionLocal()
    archive(db_session=session, item=item)
    # the caller goes on with the session after archive() swallowed the failure
    return session.query(type(item)).count()


def import_one(session: Session, item):
    try:
        session.add(item)
        session.commit()
    except Exception:
        return False
    return True


def import_and_close(item):
    # import_one's only caller closes the session right after: nothing reuses it
    with SessionLocal() as session:
        ok = import_one(session, item)
    return ok


def stage(session: Session, item):
    try:
        session.add(item)
        session.flush()
    except Exception:
        return False
    return True


def stage_or_count(item, fresh):
    with SessionLocal() as session:
        if fresh:
            stage(session, item)
            # the branch returns, so the count below never runs after a failed stage()
            return None
        return session.query(type(item)).count()

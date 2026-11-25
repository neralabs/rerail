import logging
import os
import tempfile
from collections.abc import AsyncIterator, Iterator
from contextlib import contextmanager

import asyncpg
from fastapi import HTTPException
from sqlalchemy import create_engine
from sqlalchemy.exc import SQLAlchemyError
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker, create_async_engine
from sqlalchemy.orm import Session, sessionmaker

logger = logging.getLogger(__name__)

engine = create_engine("postgresql://localhost/app")
SessionLocal = sessionmaker(autocommit=False, autoflush=False, bind=engine)
AsyncSessionLocal = async_sessionmaker(create_async_engine("postgresql+asyncpg://localhost/app"))
pool: asyncpg.Pool | None = None


# ---- cleanup not protected ---------------------------------------------------------------------------------------


# The tutorial's shape without `finally`: an error in the route skips `close()`.
def get_db() -> Iterator[Session]:
    db = SessionLocal()
    yield db
    db.close()  # caught


# Awaited release of a pooled connection.
async def get_conn() -> AsyncIterator[asyncpg.Connection]:
    conn = await pool.acquire()
    yield conn
    await pool.release(conn)  # caught


# Commit then close: skipping the commit on error is right, skipping the close leaks.
def get_db_commit() -> Iterator[Session]:
    db = SessionLocal()
    yield db
    db.commit()
    db.close()  # caught


# The handler re-raises, so the close after the `try` never runs on an error.
def get_db_rollback() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    except SQLAlchemyError:
        db.rollback()
        raise
    db.close()  # caught


# Close inside the `try` body, with a handler that cleans nothing up.
def get_db_in_body() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
        db.close()  # caught
    except Exception:
        logger.exception("request failed")
        raise


# A `finally` that does not hold the cleanup.
async def get_async_db() -> AsyncIterator[AsyncSession]:
    session = AsyncSessionLocal()
    try:
        yield session
    finally:
        logger.info("request done")
    await session.close()  # caught


# A later `try`/`finally` does not cover the `yield`.
def get_db_late_finally() -> Iterator[Session]:
    db = SessionLocal()
    yield db
    try:
        db.commit()
    finally:
        db.close()  # caught


# A temporary file: the same leak on disk.
def get_upload_dir() -> Iterator[str]:
    fd, path = tempfile.mkstemp()
    os.close(fd)
    yield path
    os.unlink(path)  # caught


# ---- swallowed exception -----------------------------------------------------------------------------------------


# Rollback without `raise`: the route's error is lost.
def get_db_swallow() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    except Exception:  # caught
        db.rollback()
    finally:
        db.close()


# A bare `except:` that only logs.
async def get_async_db_swallow() -> AsyncIterator[AsyncSession]:
    async with AsyncSessionLocal() as session:
        try:
            yield session
        except:  # caught
            logger.exception("request failed")


# Raises on one branch only.
def get_db_maybe_raise(strict: bool = True) -> Iterator[Session]:
    with SessionLocal() as db:
        try:
            yield db
        except SQLAlchemyError:  # caught
            db.rollback()
            if strict:
                raise


# A tuple with HTTPException in it, `as e`, and a return.
def get_db_tuple() -> Iterator[Session]:
    with SessionLocal() as db:
        try:
            yield db
        except (HTTPException, ValueError) as e:  # caught
            logger.warning("failed: %s", e)
            return


# ---- left alone --------------------------------------------------------------------------------------------------


def get_db_finally() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    finally:
        db.close()


def get_db_with() -> Iterator[Session]:
    with SessionLocal() as db:
        yield db


async def get_async_db_with() -> AsyncIterator[AsyncSession]:
    async with AsyncSessionLocal() as session:
        yield session


# Commit inside the `try` is skipped on an error on purpose; the handler re-raises; `finally` closes.
def get_db_tx() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
        db.commit()
    except Exception:
        db.rollback()
        raise
    finally:
        db.close()


# Commit after the yield, nothing to release: skipping it on an error is what the code wants.
def get_db_autocommit() -> Iterator[Session]:
    with SessionLocal() as db:
        yield db
        db.commit()


# The handler raises a new exception.
def get_db_translate() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    except SQLAlchemyError as e:
        db.rollback()
        raise HTTPException(status_code=500, detail="database error") from e
    finally:
        db.close()


# Every branch raises.
def get_db_branches(strict: bool = True) -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    except Exception:
        if strict:
            raise
        else:
            raise HTTPException(status_code=500)
    finally:
        db.close()


# The handler after the yield only logs and releases nothing.
def get_db_logged() -> Iterator[Session]:
    with SessionLocal() as db:
        yield db
    logger.info("request done")


# A narrow handler around the yield (not a database or broad class).
def get_db_narrow() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    except KeyboardInterrupt:
        logger.info("interrupted")
    finally:
        db.close()


def abort(code: int) -> None:
    raise HTTPException(status_code=code)


# The handler ends in a call of a helper that always raises: the error still leaves the dependency.
def get_db_abort() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    except SQLAlchemyError:
        db.rollback()
        abort(500)
    finally:
        db.close()


class AppError(Exception):
    pass


# Gap, not reported: a project's own exception class swallowed around the yield.
def get_db_app_error() -> Iterator[Session]:
    db = SessionLocal()
    try:
        yield db
    except AppError:
        pass
    finally:
        db.close()


def _release(db: Session) -> None:
    db.close()


# Gap, not reported: the release sits in a helper the generator calls after the yield.
def get_db_helper_release() -> Iterator[Session]:
    db = SessionLocal()
    yield db
    _release(db)


# Not a dependency: a contextmanager helper with the same shape (no Depends names it).
@contextmanager
def session_scope() -> Iterator[Session]:
    db = SessionLocal()
    yield db
    db.close()


# Not a dependency: a plain generator nobody passes to Depends.
def iter_sessions() -> Iterator[Session]:
    db = SessionLocal()
    yield db
    db.close()


# A class dependency whose instance another module binds and passes to Depends.
class ScopedSession:
    def __call__(self):
        db = SessionLocal()
        yield db
        db.close()  # caught


scoped_session = ScopedSession()

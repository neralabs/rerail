from typing import Annotated

import asyncpg
from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session

from .db import (
    get_async_db,
    get_async_db_swallow,
    get_async_db_with,
    get_conn,
    get_db,
    get_db_abort,
    get_db_app_error,
    get_db_autocommit,
    get_db_branches,
    get_db_commit,
    get_db_finally,
    get_db_helper_release,
    get_db_in_body,
    get_db_late_finally,
    get_db_logged,
    get_db_maybe_raise,
    get_db_narrow,
    get_db_rollback,
    get_db_swallow,
    get_db_translate,
    get_db_tuple,
    get_db_tx,
    get_db_with,
    get_upload_dir,
)

# The generators live in db.py; the Depends calls that make them dependencies live here.
SessionDep = Annotated[Session, Depends(get_db)]
ConnDep = Annotated[asyncpg.Connection, Depends(get_conn)]
AsyncSessionDep = Annotated[AsyncSession, Depends(get_async_db)]
AsyncSwallowDep = Annotated[AsyncSession, Depends(get_async_db_swallow)]
AsyncWithDep = Annotated[AsyncSession, Depends(get_async_db_with)]
UploadDirDep = Annotated[str, Depends(get_upload_dir)]

SAFE = [
    Depends(get_db_finally),
    Depends(get_db_with),
    Depends(get_db_tx),
    Depends(get_db_autocommit),
    Depends(get_db_translate),
    Depends(get_db_branches),
    Depends(get_db_logged),
    Depends(get_db_narrow),
    Depends(get_db_abort),
    # Gaps: a project's exception class, a release in a helper.
    Depends(get_db_app_error),
    Depends(get_db_helper_release),
]
UNSAFE = [
    Depends(get_db_commit),
    Depends(get_db_rollback),
    Depends(get_db_in_body),
    Depends(get_db_late_finally),
    Depends(get_db_swallow),
    Depends(get_db_maybe_raise),
    Depends(get_db_tuple),
]

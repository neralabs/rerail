from typing import Annotated

import requests
from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlmodel import Session

from .db import get_async_db, get_session

SessionDep = Annotated[Session, Depends(get_session)]
AsyncSessionDep = Annotated[AsyncSession, Depends(get_async_db)]


def get_http() -> requests.Session:
    return requests.Session()


HttpDep = Annotated[requests.Session, Depends(get_http)]

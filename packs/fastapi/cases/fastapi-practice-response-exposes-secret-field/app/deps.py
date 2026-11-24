from collections.abc import Generator
from typing import Annotated

from fastapi import Depends
from sqlalchemy.orm import Session

from app.models import User


def get_db() -> Generator[Session, None, None]:
    with Session() as session:
        yield session


SessionDep = Annotated[Session, Depends(get_db)]


def get_current_user(session: SessionDep) -> User:
    return session.get(User, 1)


CurrentUser = Annotated[User, Depends(get_current_user)]

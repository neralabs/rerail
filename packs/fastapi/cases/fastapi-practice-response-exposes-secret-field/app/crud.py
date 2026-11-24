from typing import Optional

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.models import Integration, User


def get_user_by_email(session: Session, email: str) -> Optional[User]:
    return session.scalars(select(User).where(User.email == email)).first()


def list_integrations(session: Session) -> list[Integration]:
    return list(session.exec(select(Integration)).all())


def count_users(session: Session) -> int:
    return session.query(User).count()

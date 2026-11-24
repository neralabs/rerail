import enum

from sqlalchemy import Column, Integer, String
from sqlalchemy.orm import Mapped, mapped_column

from app.models.base import AuditedBase, Base, TimestampMixin
from app.models.types import GUID

ADMIN_ROLES = ("owner", "admin")


class UserRole(enum.Enum):
    OWNER = "owner"
    MEMBER = "member"


class User(TimestampMixin, Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)
    email: Mapped[str] = mapped_column(String(255))
    role = Column(String(20))


class AdminUser(User):
    # Single-table inheritance: mapped onto users.
    __mapper_args__ = {"polymorphic_identity": "admin"}


class Group(AuditedBase):
    # No __tablename__: the base's declared_attr names it "group".
    id = Column(GUID, primary_key=True)
    slug = Column(String(80))


class UserQuery:
    # A plain helper class: not mapped.
    def by_email(self, session, email):
        return session.query(User).filter_by(email=email).one()

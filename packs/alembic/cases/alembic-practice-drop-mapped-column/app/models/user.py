from sqlalchemy import Column, ForeignKey, Integer, String, Table, Text
from sqlalchemy.orm import Mapped, column_property, deferred, mapped_column

from app.models.base import Base, SoftDeleteMixin, TokenMixin


class User(SoftDeleteMixin, TokenMixin, Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)
    nick = Column(String(40))
    email: Mapped[str] = mapped_column(String(255))
    display = Column("display_name", String(80))
    handle: Mapped[str]
    bio = deferred(Column(Text))
    locale = Column(String(10))
    avatar_url: Mapped[str] = mapped_column(Text)
    signature = column_property(Column("sig", Text))
    timezone = Column(String(40), name="tz")
    legacy_token = None  # the mixin's column, no longer mapped (expand/contract)


class Order(Base):
    __tablename__ = "orders"
    id: Mapped[int] = mapped_column(primary_key=True)
    user_id: Mapped[int] = mapped_column(ForeignKey("users.id"))
    status: Mapped[str]


class Tag(Base):
    __tablename__ = "tags"
    id: Mapped[int] = mapped_column(primary_key=True)


# A class mapped onto a Table object, and an association table.
class Legacy(Base):
    __table__ = Table("legacy", Base.metadata, Column("id", Integer, primary_key=True), Column("code", String))


user_tags = Table(
    "user_tags",
    Base.metadata,
    Column("user_id", ForeignKey("users.id"), primary_key=True),
    Column("tag_id", ForeignKey("tags.id"), primary_key=True),
)

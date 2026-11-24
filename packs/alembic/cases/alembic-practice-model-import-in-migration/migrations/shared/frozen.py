"""Models frozen for the migrations (superset's migrations/shared/security_converge.py): they never follow the app."""
import sqlalchemy as sa
from sqlalchemy.orm import declarative_base

FrozenBase = declarative_base()


class Permission(FrozenBase):
    __tablename__ = "ab_permission"
    id = sa.Column(sa.Integer, primary_key=True)
    name = sa.Column(sa.String(100))

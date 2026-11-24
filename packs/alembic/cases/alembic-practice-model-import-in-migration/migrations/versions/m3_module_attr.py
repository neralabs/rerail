"""a model module imported whole, read through its attributes (jupyterhub's 651f5419b74d)"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.orm import Session

from app.models import user as user_models
from app.models import types

revision = "m3"
down_revision = "m2"
branch_labels = None


def describe(user: user_models.User) -> str:
    return user.email


def upgrade():
    op.add_column("users", sa.Column("prefs", types.JSONDict()))
    db = Session(bind=op.get_bind())
    for user in db.query(user_models.User):  # caught
        user.role = user_models.UserRole.MEMBER.value
    db.query(user_models.User).filter_by(role=None).delete()
    db.query(user_models.Group).delete()  # caught


def downgrade():
    op.drop_column("users", "prefs")

"""the safe idioms: a table declared in the migration, a model class of its own, a frozen shared model"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.orm import Session, declarative_base

from migrations.shared.frozen import Permission

revision = "m4"
down_revision = "m3"
branch_labels = None

users = sa.table("users", sa.column("id", sa.Integer), sa.column("role", sa.String))

LocalBase = declarative_base()


class Group(LocalBase):
    __tablename__ = "group"
    id = sa.Column(sa.Integer, primary_key=True)
    slug = sa.Column(sa.String(80))


def upgrade():
    op.execute(users.update().where(users.c.role.is_(None)).values(role="member"))
    session = Session(bind=op.get_bind())
    for group in session.query(Group):
        group.slug = str(group.id)
    session.query(Permission).filter_by(name="can_read").delete()


def downgrade():
    pass

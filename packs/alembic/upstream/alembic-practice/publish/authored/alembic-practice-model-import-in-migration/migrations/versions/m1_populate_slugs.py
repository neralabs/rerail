"""populate group slugs (mealie's 04ac51cbe9a4)"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.orm import Session

from app.models.user import (
    AdminUser,  # caught
    Group,  # caught
    UserRole,
    ADMIN_ROLES,
)
from app.models.types import GUID, JSONDict
from app.models.base import Base, TimestampMixin

revision = "m1"
down_revision = None
branch_labels = None


def populate_slugs(session: Session):
    for group in session.query(Group).all():
        group.slug = group.name.lower()
    session.query(AdminUser).filter(AdminUser.role.in_(ADMIN_ROLES)).update({"role": UserRole.OWNER.value})


def upgrade():
    op.add_column("group", sa.Column("slug", sa.String(80)))
    op.add_column("group", sa.Column("settings", JSONDict()))
    op.add_column("group", sa.Column("owner_id", GUID()))
    populate_slugs(Session(bind=op.get_bind()))


def downgrade():
    op.drop_column("group", "slug")

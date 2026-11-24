"""more import shapes"""
from typing import TYPE_CHECKING

import sqlalchemy as sa
from alembic import op
from sqlalchemy.orm import Session

from app.models import User  # caught
from app.models.hero import Hero  # caught
from app.models.hero import HeroBase, HeroPublic
from app.legacy import Invoice as LegacyInvoice  # caught
from app.legacy import LegacyBase
from app.models.user import UserQuery
from app.schemas import UserSchema

if TYPE_CHECKING:
    from app.models.user import AdminUser

revision = "m2"
down_revision = "m1"
branch_labels = None


def upgrade():
    session = Session(bind=op.get_bind())
    for invoice in session.query(LegacyInvoice):
        invoice.total = invoice.total or 0
    session.query(User).update({"role": "member"})
    session.query(Hero).delete()


def downgrade():
    from app.models.user import Group  # caught

    Session(bind=op.get_bind()).query(Group).delete()

"""a model in a namespace package (Netflix/dispatch's 3c49f62d7914)"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.orm import Session

from app.plugins.billing.models import Payment  # caught

revision = "m5"
down_revision = "m4"
branch_labels = None


def upgrade():
    op.add_column("payment", sa.Column("currency", sa.String(3)))
    Session(bind=op.get_bind()).query(Payment).update({"currency": "USD"})


def downgrade():
    op.drop_column("payment", "currency")

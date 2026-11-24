"""invoices: lock_timeout set first (polar's form)"""
import sqlalchemy as sa
from alembic import op

revision = "c3"
down_revision = "c2"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SET LOCAL lock_timeout = '5s'")
    op.add_column("invoices", sa.Column("due_at", sa.DateTime(), nullable=True))
    op.create_unique_constraint("uq_invoices_number", "invoices", ["number"])
    op.drop_table("invoice_drafts")


def downgrade():
    op.drop_column("invoices", "due_at")

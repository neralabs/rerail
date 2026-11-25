"""ledger: models reached through its metadata module's imports, in another top-level package"""
from alembic import op

revision = "l1"
down_revision = None


def upgrade():
    op.drop_column("entries", "memo")  # caught
    op.drop_column("audit", "actor")  # caught
    op.drop_column("journal", "ref")  # caught
    op.drop_column("users", "nick")  # mapped only by the app and billing trees
    op.drop_column("invoices", "memo")  # mapped only by billing


def downgrade():
    pass

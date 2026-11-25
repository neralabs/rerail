"""recreate a column under its own name (left alone), and a rename that widens VARCHAR(32) to 64 (reported)"""
import sqlalchemy as sa
from alembic import op

revision = "b11"
down_revision = "b10"
branch_labels = None
depends_on = None


def upgrade():
    op.drop_column("orders", "status")
    op.add_column("orders", sa.Column("status", sa.String(20)))
    op.drop_column("items", "sku")  # caught
    op.add_column("items", sa.Column("code", sa.String(64)))


def downgrade():
    op.add_column("items", sa.Column("sku", sa.VARCHAR(length=32)))
    op.drop_column("items", "code")
    op.drop_column("orders", "status")
    op.add_column("orders", sa.Column("status", sa.VARCHAR(length=20)))

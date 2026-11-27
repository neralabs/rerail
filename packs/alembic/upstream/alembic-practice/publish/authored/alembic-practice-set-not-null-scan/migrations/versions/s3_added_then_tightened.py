"""add a column nullable, backfill it, then make it NOT NULL"""
import sqlalchemy as sa
from alembic import op

from migrations.shared.utils import add_columns

revision = "s3"
down_revision = "s2"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("tier", sa.String(20), nullable=True))
    op.execute("UPDATE users SET tier = 'free'")
    op.alter_column("users", "tier", nullable=False)  # caught

    with op.batch_alter_table("orders") as batch_op:
        batch_op.add_column(sa.Column("currency", sa.String(3)))
    op.execute("UPDATE orders SET currency = 'USD'")
    with op.batch_alter_table("orders") as batch_op:
        batch_op.alter_column("currency", existing_type=sa.String(3), nullable=False)  # caught

    # A project's own add_columns wrapper.
    add_columns("themes", sa.Column("is_default", sa.Boolean(), nullable=True))
    op.execute("UPDATE themes SET is_default = false")
    op.alter_column("themes", "is_default", nullable=False)  # caught


def downgrade():
    op.drop_column("orders", "currency")
    op.drop_column("users", "tier")

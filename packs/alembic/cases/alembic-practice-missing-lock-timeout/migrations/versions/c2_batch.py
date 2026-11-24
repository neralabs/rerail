"""orders: a batch drop_column"""
import sqlalchemy as sa
from alembic import op

revision = "c2"
down_revision = "c1"
branch_labels = None
depends_on = None


def upgrade():
    # Data first: an UPDATE takes row locks only.
    op.execute("UPDATE orders SET note = '' WHERE note IS NULL")
    with op.batch_alter_table("orders", schema=None) as batch_op:
        batch_op.drop_column("legacy_ref")  # caught
        batch_op.create_check_constraint("ck_orders_total", "total >= 0")


def downgrade():
    with op.batch_alter_table("orders", schema=None) as batch_op:
        batch_op.add_column(sa.Column("legacy_ref", sa.String(40)))

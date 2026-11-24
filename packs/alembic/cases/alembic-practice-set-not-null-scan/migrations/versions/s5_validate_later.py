"""the later revision of the safe sequence: validate the CHECK, then SET NOT NULL"""
import sqlalchemy as sa
from alembic import op

revision = "s5"
down_revision = "s4"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("ALTER TABLE refunds VALIDATE CONSTRAINT ck_refunds_reason_not_null")
    op.alter_column("refunds", "reason", existing_type=sa.String(), nullable=False)
    op.drop_constraint("ck_refunds_reason_not_null", "refunds", type_="check")

    # A foreign key's validation says nothing about this column.
    op.execute("ALTER TABLE refunds VALIDATE CONSTRAINT refunds_order_id_fkey")
    op.alter_column("refunds", "amount", existing_type=sa.Integer(), nullable=False)  # caught

    # A NOT VALID check validated in the same upgrade, linked by its constant name.
    op.create_check_constraint(CHECK_NAME, "refunds", "price IS NOT NULL", postgresql_not_valid=True)
    op.execute(f"ALTER TABLE refunds VALIDATE CONSTRAINT {CHECK_NAME}")
    op.alter_column("refunds", "price", nullable=False)


CHECK_NAME = "ck_refunds_p_nn"


def downgrade():
    op.alter_column("refunds", "reason", nullable=True)

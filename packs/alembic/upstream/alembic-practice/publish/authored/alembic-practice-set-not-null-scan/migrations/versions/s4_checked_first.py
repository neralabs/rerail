"""SET NOT NULL behind a validated CHECK (... IS NOT NULL): no scan"""
import sqlalchemy as sa
from alembic import op

revision = "s4"
down_revision = "s3"
branch_labels = None
depends_on = None

NOT_NULL_CHECK = "payments_status_not_null_check"


def upgrade():
    with op.get_context().autocommit_block():
        op.execute(
            f"ALTER TABLE payments ADD CONSTRAINT {NOT_NULL_CHECK} "
            "CHECK (status IS NOT NULL) NOT VALID"
        )
        op.execute(f"ALTER TABLE payments VALIDATE CONSTRAINT {NOT_NULL_CHECK}")
        op.alter_column("payments", "status", existing_type=sa.String(), nullable=False)
        op.execute(f"ALTER TABLE payments DROP CONSTRAINT {NOT_NULL_CHECK}")

    # A CHECK added without NOT VALID is validated as it is added (its own scan is another rule's concern).
    op.create_check_constraint("ck_payments_currency_not_null", "payments", "currency IS NOT NULL")
    op.alter_column("payments", "currency", nullable=False)

    # NOT VALID and never validated: SET NOT NULL still scans.
    op.create_check_constraint(
        "ck_payments_paid_at_not_null", "payments", "paid_at IS NOT NULL", postgresql_not_valid=True
    )
    op.alter_column("payments", "paid_at", nullable=False)  # caught


def downgrade():
    op.alter_column("payments", "status", nullable=True)

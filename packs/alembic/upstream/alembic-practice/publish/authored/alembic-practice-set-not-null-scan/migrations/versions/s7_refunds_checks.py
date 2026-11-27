"""the first revision of the safe sequence: NOT VALID checks that s5 validates"""
from alembic import op

revision = "s7"
down_revision = "s3"
branch_labels = None
depends_on = None


def upgrade():
    op.create_check_constraint(
        "ck_refunds_reason_not_null", "refunds", "reason IS NOT NULL", postgresql_not_valid=True
    )
    op.execute("ALTER TABLE refunds ADD CONSTRAINT ck_refunds_note_nn CHECK (note IS NOT NULL) NOT VALID")


def downgrade():
    op.drop_constraint("ck_refunds_note_nn", "refunds", type_="check")
    op.drop_constraint("ck_refunds_reason_not_null", "refunds", type_="check")

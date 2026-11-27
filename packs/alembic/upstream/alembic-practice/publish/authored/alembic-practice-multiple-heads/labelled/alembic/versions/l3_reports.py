"""root of the reports branch"""
import sqlalchemy as sa
from alembic import op

revision = "l3"
down_revision = "l1"
branch_labels = ("reports",)

def upgrade():
    op.create_table("reports", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass

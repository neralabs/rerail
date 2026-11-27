"""reports branch"""
import sqlalchemy as sa
from alembic import op

revision = "l4"
down_revision = "l3"
branch_labels = None

def upgrade():
    op.create_table("report_rows", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass

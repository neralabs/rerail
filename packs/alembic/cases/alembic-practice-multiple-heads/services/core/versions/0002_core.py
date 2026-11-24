"""core step 2"""
import sqlalchemy as sa
from alembic import op

revision = "0002"
down_revision = "0001"
branch_labels = None

def upgrade():
    op.create_table("core_2", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass

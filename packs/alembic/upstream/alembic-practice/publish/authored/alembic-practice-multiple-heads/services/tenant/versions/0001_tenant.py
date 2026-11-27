"""tenant step 1"""
import sqlalchemy as sa
from alembic import op

revision = "0001"
down_revision = None
branch_labels = None

def upgrade():
    op.create_table("tenant_1", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass

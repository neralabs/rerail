"""tenant step 3"""
import sqlalchemy as sa
from alembic import op

revision = "0003"
down_revision = "0002"
branch_labels = None

def upgrade():
    op.create_table("tenant_3", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass

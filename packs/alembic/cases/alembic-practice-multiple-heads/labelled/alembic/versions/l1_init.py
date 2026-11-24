"""init"""
import sqlalchemy as sa
from alembic import op

revision = "l1"
down_revision = None
branch_labels = None

def upgrade():
    op.create_table("users", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass

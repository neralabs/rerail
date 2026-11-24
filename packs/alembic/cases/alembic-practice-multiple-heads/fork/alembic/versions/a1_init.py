"""init (the base carries the tree's label)"""
import sqlalchemy as sa
from alembic import op

revision = "a1"
down_revision = None
branch_labels = ("app",)
depends_on = None

def upgrade():
    op.create_table("users", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass

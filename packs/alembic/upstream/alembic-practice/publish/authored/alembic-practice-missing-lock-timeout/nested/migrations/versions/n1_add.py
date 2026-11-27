"""nested: the outer tree, covered by its env.py"""
import sqlalchemy as sa
from alembic import op

revision = "n1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("n1", sa.Integer(), nullable=True))


def downgrade():
    pass

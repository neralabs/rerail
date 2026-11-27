"""nested: the tenant tree runs under its own env.py"""
import sqlalchemy as sa
from alembic import op

revision = "n2"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("projects", sa.Column("n2", sa.Integer(), nullable=True))


def downgrade():
    pass

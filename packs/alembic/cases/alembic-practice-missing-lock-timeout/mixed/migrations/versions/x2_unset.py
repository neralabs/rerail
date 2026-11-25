"""mixed: two do not"""
import sqlalchemy as sa
from alembic import op

revision = "x2"
down_revision = 'x1'
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("orders", sa.Column("x2", sa.Integer(), nullable=True))


def downgrade():
    pass

"""main line"""
import sqlalchemy as sa
from alembic import op

revision = "l2"
down_revision = "l1"
branch_labels = None

def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    pass

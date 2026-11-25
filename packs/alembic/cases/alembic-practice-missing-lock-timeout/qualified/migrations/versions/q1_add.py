"""env.py reaches alembic.context through `import alembic.context`"""
import sqlalchemy as sa
from alembic import op

revision = "q1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("q1", sa.Integer(), nullable=True))


def downgrade():
    pass

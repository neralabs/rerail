"""plain: RESET sets nothing"""
import sqlalchemy as sa
from alembic import op

revision = "p2"
down_revision = 'p1'
branch_labels = None
depends_on = None


def upgrade():
    op.execute("RESET lock_timeout")
    op.drop_column("users", "nick")


def downgrade():
    pass

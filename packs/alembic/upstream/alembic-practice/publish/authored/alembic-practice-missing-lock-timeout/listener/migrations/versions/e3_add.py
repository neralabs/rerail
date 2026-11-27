"""a connect hook of the engine's module runs SET lock_timeout"""
import sqlalchemy as sa
from alembic import op

revision = "e3"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("nick", sa.String(40), nullable=True))


def downgrade():
    pass

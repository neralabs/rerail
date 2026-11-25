"""plain: a column on an existing table"""
import sqlalchemy as sa
from alembic import op

revision = "p1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("nick", sa.String(40), nullable=True))
    op.create_index("ix_users_nick", "users", ["nick"])


def downgrade():
    pass

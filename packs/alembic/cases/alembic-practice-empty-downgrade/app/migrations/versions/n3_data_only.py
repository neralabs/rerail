"""backfill users.email"""
import sqlalchemy as sa
from alembic import op

revision = "n3"
down_revision = "n2"
branch_labels = None
depends_on = None

users = sa.table("users", sa.column("id", sa.Integer()), sa.column("email", sa.String()))


def upgrade():
    conn = op.get_bind()
    op.execute("UPDATE users SET email = lower(email)")
    op.bulk_insert(users, [{"id": 1, "email": "admin@example.com"}])


def downgrade():
    pass

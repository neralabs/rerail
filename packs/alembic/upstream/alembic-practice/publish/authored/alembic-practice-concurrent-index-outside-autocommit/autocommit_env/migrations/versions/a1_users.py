"""an env.py that runs revisions without a transaction"""
from alembic import op

revision = "a1"
down_revision = None
branch_labels = None


def upgrade():
    op.create_index("ix_users_email", "users", ["email"], postgresql_concurrently=True)


def downgrade():
    op.drop_index("ix_users_email", table_name="users", postgresql_concurrently=True)

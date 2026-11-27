"""a concurrent index under an env.py whose engine runs in AUTOCOMMIT"""
from alembic import op

revision = "i1"
down_revision = None
branch_labels = None


def upgrade():
    op.create_index("ix_users_email", "users", ["email"], postgresql_concurrently=True)


def downgrade():
    op.drop_index("ix_users_email", table_name="users", postgresql_concurrently=True)

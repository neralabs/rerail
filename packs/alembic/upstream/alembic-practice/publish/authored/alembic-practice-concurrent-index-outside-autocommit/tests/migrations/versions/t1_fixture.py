"""a migration the test suite runs against a throwaway database"""
from alembic import op

revision = "t1"
down_revision = None
branch_labels = None


def upgrade():
    op.create_index("ix_users_email", "users", ["email"], postgresql_concurrently=True)


def downgrade():
    op.drop_index("ix_users_email", table_name="users", postgresql_concurrently=True)

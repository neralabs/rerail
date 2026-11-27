"""a revision under no env.py's directory (version_locations elsewhere): the run's only non-transactional env.py runs it"""
from alembic import op

revision = "a2"
down_revision = "a1"
branch_labels = None


def upgrade():
    op.create_index("ix_users_handle", "users", ["handle"], postgresql_concurrently=True)


def downgrade():
    op.drop_index("ix_users_handle", table_name="users", postgresql_concurrently=True)

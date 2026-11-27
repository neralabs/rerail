"""an operator script outside any revision: not a migration"""
from alembic import op


def upgrade():
    op.create_index("ix_users_email", "users", ["email"])

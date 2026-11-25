"""CONCURRENTLY switched on through constants"""
from alembic import op as alembic_op

from app.db import CONCURRENT_INDEXES

revision = "c5"
down_revision = "c4"
branch_labels = None

CONCURRENT = True
PLAIN = False


def upgrade():
    alembic_op.create_index("ix_users_city", "users", ["city"], postgresql_concurrently=CONCURRENT)  # caught
    alembic_op.create_index("ix_users_zip", "users", ["zip"], postgresql_concurrently=CONCURRENT_INDEXES)  # caught
    alembic_op.create_index("ix_users_state", "users", ["state"], postgresql_concurrently=PLAIN)
    with alembic_op.get_context().autocommit_block():
        alembic_op.create_index("ix_users_country", "users", ["country"], postgresql_concurrently=CONCURRENT)


def downgrade():
    pass

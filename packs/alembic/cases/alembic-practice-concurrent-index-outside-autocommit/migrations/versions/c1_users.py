"""concurrent indexes on users"""
from alembic import op

revision = "c1"
down_revision = None
branch_labels = None

INDEX = "ix_users_created_at"


def upgrade():
    op.create_index("ix_users_email", "users", ["email"], postgresql_concurrently=True)  # caught
    op.create_index(  # caught
        INDEX,
        "users",
        ["created_at"],
        unique=False,
        postgresql_concurrently=True,
    )
    op.drop_index("ix_users_old", table_name="users", postgresql_concurrently=True)  # caught

    # The fix: outside the migration's transaction.
    with op.get_context().autocommit_block():
        op.drop_index("ix_users_name", table_name="users", if_exists=True, postgresql_concurrently=True)
        op.create_index("ix_users_name", "users", ["name"], postgresql_concurrently=True)
        if INDEX:
            op.create_index("ix_users_nick", "users", ["nick"], postgresql_concurrently=True)

    # After the block, the revision's transaction is open again.
    op.create_index("ix_users_team", "users", ["team_id"], postgresql_concurrently=True)  # caught

    # Not concurrent: alembic-practice-index-not-concurrent's concern, not this rule's.
    op.create_index("ix_users_plain", "users", ["plain"])
    op.create_index("ix_users_off", "users", ["off"], postgresql_concurrently=False)

    _org_indexes()


def _org_indexes():
    op.create_index("ix_users_org_id", "users", ["org_id"], postgresql_concurrently=True)  # caught


def downgrade():
    op.drop_index("ix_users_email", table_name="users", postgresql_concurrently=True)  # caught
    with op.get_context().autocommit_block():
        op.drop_index(INDEX, table_name="users", postgresql_concurrently=True)

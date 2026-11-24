"""user indexes"""
import sqlalchemy as sa
from alembic import op

revision = "c1"
down_revision = None
branch_labels = None

USERS = "users"
INDEX = "ix_users_created_at"


def upgrade():
    op.create_index("ix_users_email", "users", ["email"])  # caught
    op.create_index(op.f("ix_users_handle"), table_name="users", columns=["handle"], unique=True)  # caught
    op.create_index(INDEX, USERS, ["created_at"])  # caught
    op.create_index(  # caught
        "ix_users_team_id",
        "users",
        ["team_id"],
        postgresql_concurrently=False,
    )
    op.create_index(index_name="ix_users_kw", table_name="users", columns=["kw"])  # caught

    # A table this upgrade() creates is empty: its indexes lock nothing.
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("name", sa.String(40)))
    op.create_index("ix_tags_name", "tags", ["name"])
    op.create_index(op.f("ix_tags_id"), "tags", ["id"], unique=True)

    # The concurrent build, inside an autocommit block, after clearing an INVALID leftover.
    with op.get_context().autocommit_block():
        op.drop_index("ix_users_name", table_name="users", if_exists=True, postgresql_concurrently=True)
        op.create_index("ix_users_name", "users", ["name"], postgresql_concurrently=True)

    # Concurrent but outside an autocommit block: another rule's concern, not this one's.
    op.create_index("ix_users_nick", "users", ["nick"], postgresql_concurrently=True)

    _add_team_indexes()


def _add_team_indexes():
    op.create_index("ix_users_org_id", "users", ["org_id"])  # caught


def downgrade():
    # Rollbacks re-create indexes on the rare path: left alone.
    op.create_index("ix_users_legacy", "users", ["legacy"])
    op.drop_index("ix_users_email", table_name="users")

"""search index: not an Alembic operation"""
from app.search import op

revision = "c4"
down_revision = "c3"
branch_labels = None


def upgrade():
    # `op` here is the search client's, not alembic.op.
    op.create_index("users", "users", ["email"])


def downgrade():
    op.delete_index("users")

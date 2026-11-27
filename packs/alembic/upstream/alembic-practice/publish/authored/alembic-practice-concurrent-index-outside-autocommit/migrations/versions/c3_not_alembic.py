"""search index: not an Alembic operation"""
from app.search import op

revision = "c3"
down_revision = "c2"
branch_labels = None


def upgrade():
    # `op` here is the search client's, not alembic.op.
    op.create_index("users", "users", ["email"], postgresql_concurrently=True)


def downgrade():
    pass

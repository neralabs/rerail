"""dialect tests read by their structure: PostgreSQL runs these, reported"""
from alembic import op

from migrations.shared.dialects import is_postgres, postgres_enabled

revision = "c10"
down_revision = "c9"
branch_labels = None


def upgrade():
    bind = op.get_bind()
    settings = object()
    readonly = False
    dialect_name = bind.dialect.name
    # PostgreSQL runs these: reported.
    if is_postgres():
        op.create_index("ix_p1", "p_one", ["a"])  # caught
    if not settings.USE_SQLITE_FOR_TESTS:
        op.create_index("ix_p2", "p_two", ["a"])  # caught
    url = str(bind.engine.url)
    if "postgres" in url and not readonly:
        op.create_index("ix_p3", "p_three", ["a"])  # caught
    if postgres_enabled(settings):
        pass
    else:
        op.create_index("ix_p4", "p_four", ["a"])  # caught
    if dialect_name == "sqlite" or readonly:
        pass
    else:
        op.create_index("ix_p5", "p_five", ["a"])  # caught


def downgrade():
    pass

"""dialect tests read by their structure"""
from alembic import op
from sqlalchemy.dialects import mysql, postgresql

from migrations.shared.dialects import is_mysql, is_postgres, postgres_enabled

revision = "c9"
down_revision = "c8"
branch_labels = None


def upgrade():
    bind = op.get_bind()
    settings = object()
    readonly = False
    # Other databases only: left alone.
    if is_mysql():
        op.create_index("ix_m1", "m_one", ["a"])
    if not is_postgres():
        op.create_index("ix_m2", "m_two", ["a"])
    if isinstance(bind.dialect, mysql.dialect):
        op.create_index("ix_m3", "m_three", ["a"])
    dialect_name = bind.dialect.name
    if dialect_name in ("sqlite", "mysql"):
        op.create_index("ix_m4", "m_four", ["a"])
    elif dialect_name == "mssql":
        op.create_index("ix_m5", "m_five", ["a"])
    is_sqlite = bind.dialect.name == "sqlite"
    if is_sqlite:
        op.create_index("ix_m6", "m_six", ["a"])
    if isinstance(bind.dialect, postgresql.dialect):
        pass
    else:
        op.create_index("ix_m7", "m_seven", ["a"])
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

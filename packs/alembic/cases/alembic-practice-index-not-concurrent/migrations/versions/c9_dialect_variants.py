"""dialect tests read by their structure: other databases only, left alone"""
from alembic import op
from sqlalchemy.dialects import mysql, postgresql

from migrations.shared.dialects import is_mysql, is_postgres

revision = "c9"
down_revision = "c8"
branch_labels = None


def upgrade():
    bind = op.get_bind()
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


def downgrade():
    pass

"""a table created here, and dialect branches"""
import sqlalchemy as sa
from alembic import op

revision = "s2"
down_revision = "s1"
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("name", sa.String()))
    # tags is new and empty: nothing to scan.
    op.alter_column("tags", "name", nullable=False)
    with op.batch_alter_table("tags") as batch_op:
        batch_op.alter_column("id", nullable=False)

    conn = op.get_bind()
    if conn.dialect.name == "postgresql":
        op.alter_column("invoices", "number", nullable=False)  # caught
    else:
        op.alter_column("invoices", "number", nullable=False)
    if conn.dialect.name == "sqlite":
        with op.batch_alter_table("invoices") as batch_op:
            batch_op.alter_column("due_at", nullable=False)


def downgrade():
    op.drop_table("tags")

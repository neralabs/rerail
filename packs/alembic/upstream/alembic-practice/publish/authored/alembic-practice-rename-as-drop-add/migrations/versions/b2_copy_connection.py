"""copy through the connection"""
import sqlalchemy as sa
from alembic import op

revision = "b2"
down_revision = "b1"
branch_labels = None
depends_on = None


def upgrade():
    conn = op.get_bind()
    op.add_column("chat", sa.Column("chat_json", sa.Text()))
    rows = conn.execute(sa.text("SELECT id, old_chat FROM chat")).fetchall()
    for row in rows:
        conn.execute(sa.text("UPDATE chat SET chat_json = :v WHERE id = :id"), {"v": row.old_chat, "id": row.id})
    op.drop_column("chat", "old_chat")


def downgrade():
    op.add_column("chat", sa.Column("old_chat", sa.TEXT()))
    op.drop_column("chat", "chat_json")

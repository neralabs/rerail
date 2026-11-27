"""a Flask-SQLAlchemy model, and models imported only for their table names"""
import sqlalchemy as sa
from alembic import op

from app.models.hero import Hero  # caught
from app.models.user import User  # caught
from app.models.user import Group  # caught
from app.widgets import Gadget, Widget  # caught

revision = "m6"
down_revision = "m5"
branch_labels = None


def upgrade():
    op.add_column(User.__tablename__, sa.Column("nick", sa.String(40), nullable=True))
    op.add_column(Group.__table__.name, sa.Column("slug", sa.String(40), nullable=True))
    session = sa.orm.Session(bind=op.get_bind())
    for widget in session.query(Widget):
        widget.color = widget.color or "grey"
    op.add_column(Hero.__tablename__, sa.Column("alias", sa.String(40), nullable=True))
    session.query(Hero).count()
    print(Gadget.name)


def downgrade():
    pass

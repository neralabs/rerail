from sqlalchemy import update

from app.models import Item, Note


def test_relabel(session):
    session.execute(update(Item).where(Item.label == "a").values(label="b"))
    session.execute(update(Note).values(title="x"))
    assert session.query(Item).filter(Item.label == "b").count() == 0

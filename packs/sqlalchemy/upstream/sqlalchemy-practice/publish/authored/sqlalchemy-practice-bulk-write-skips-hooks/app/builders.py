from sqlalchemy import select, update

from app.models import Item, Note


def build_relabel(new):
    return update(Item).values(label=new)


def build_relabel_named(new):
    stmt = update(Item).where(Item.label.is_not(None))
    stmt = stmt.values(label=new)
    return stmt


def build_retitle(new):
    return update(Note).values(title=new)


def build_query():
    return select(Item)


def build_sql(new):
    return f"UPDATE item SET label = '{new}'"

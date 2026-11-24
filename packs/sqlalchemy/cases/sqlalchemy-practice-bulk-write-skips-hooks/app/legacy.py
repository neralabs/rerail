from app.db import db
from app.models import Account, Item, Note, Tag


def deactivate(ids):
    Account.query.filter(Account.id.in_(ids)).update({"active": False})  # caught
    db.session.query(Item).filter(Item.id.in_(ids)).delete()  # caught
    db.session.query(Note).filter(Note.id.in_(ids)).delete()
    db.session.query(Tag).filter(Tag.id.in_(ids)).update({"color": "grey"})
    db.session.query(Tag).filter(Tag.id.in_(ids)).update({Tag.name: "archived"}, synchronize_session=False)  # caught


def merge_settings(settings, extra):
    settings.update(extra)
    return settings

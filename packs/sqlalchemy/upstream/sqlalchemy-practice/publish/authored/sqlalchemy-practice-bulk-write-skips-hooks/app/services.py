from sqlalchemy import Column, Integer, MetaData, String, Table, delete, insert, update
from sqlalchemy.orm import Session

from app.models import AuditedDoc, Dataset, Invoice, Item, MonthlyReport, Note, Report, Tag, Ticket, Unit

metadata = MetaData()
legacy_items = Table("legacy_item", metadata, Column("id", Integer), Column("label", String))


def relabel(session: Session, old, new):
    session.execute(update(Item).where(Item.label == old).values(label=new))  # caught


def relabel_stmt(session: Session, old, new):
    stmt = update(Item).where(Item.label == old).values(label=new)
    session.execute(stmt)  # caught


def drop_items(session: Session, list_id):
    session.execute(delete(Item).where(Item.list_id == list_id))  # caught


def add_items(session: Session, rows):
    session.execute(insert(Item), rows)  # caught


def save_items(session: Session, labels):
    session.bulk_save_objects([Item(label=label) for label in labels])  # caught


def relabel_legacy(session: Session, old, new):
    session.execute(update(legacy_items).where(legacy_items.c.label == old).values(label=new))


def rename_notes(session: Session, old, new):
    session.execute(update(Note).where(Note.title == old).values(title=new))


def recolor_tags(session: Session, color):
    session.execute(update(Tag).values(color=color))


def rename_tags(session: Session, old, new):
    session.execute(update(Tag).where(Tag.name == old).values({"name": new}))  # caught


def rename_tags_by_attribute(session: Session, old, new):
    session.execute(update(Tag).where(Tag.name == old).values({Tag.name: new}))  # caught


def retag(session: Session, values):
    session.execute(update(Tag).values(**values))  # caught


def delete_tags(session: Session):
    session.execute(delete(Tag))


def import_tags(session: Session, rows):
    session.bulk_insert_mappings(Tag, rows)  # caught


def save_tags(session: Session, names):
    session.bulk_save_objects([Tag(name=name) for name in names])


def rename_units(session: Session, old, new):
    session.execute(update(Unit).where(Unit.name == old).values(name=new))  # caught


def reabbreviate_units(session: Session, abbreviation):
    session.execute(update(Unit).values(abbreviation=abbreviation))


def sync_units(session: Session, rows):
    session.bulk_update_mappings(Unit, rows)  # caught


def close_tickets(session: Session):
    session.execute(update(Ticket).values(closed=True))  # caught


def delete_tickets(session: Session):
    session.execute(delete(Ticket))


def purge_invoices(session: Session):
    session.execute(delete(Invoice))  # caught


def purge_reports(session: Session):
    session.execute(delete(Report).where(Report.kind == "report"))  # caught


def purge_monthly_reports(session: Session):
    session.execute(delete(MonthlyReport))


def clear_docs(session: Session):
    session.execute(update(AuditedDoc).values(body=""))


def rename_datasets(session: Session, name):
    session.execute(update(Dataset).values(name=name))  # caught

from sqlalchemy import event, update
from sqlalchemy.orm import Session

from app.models import Account, Audited, AuditedDoc, Item, Report, Unit


@event.listens_for(Item, "after_insert")
@event.listens_for(Item, "after_update")
@event.listens_for(Item, "after_delete")
def touch_list(mapper, connection, target):
    connection.info.setdefault("touched_lists", set()).add(target.list_id)


@event.listens_for(Unit.name, "set")
def unit_name_set(target, value, oldvalue, initiator):
    target.normalized_name = (value or "").lower()


def bump_account(mapper, connection, target):
    connection.execute(update(Account).where(Account.id == target.id).values(version=Account.version + 1))


event.listen(Account, "after_update", bump_account)


@event.listens_for(Audited, "before_delete", propagate=True)
def archive(mapper, connection, target):
    connection.info.setdefault("archived", []).append((type(target).__name__, target.id))


@event.listens_for(Report, "after_delete")
def drop_report_files(mapper, connection, target):
    connection.info.setdefault("report_files", []).append(target.id)


@event.listens_for(AuditedDoc, "after_update")
def log_doc(mapper, connection, target):
    connection.info.setdefault("doc_log", []).append(target.id)


@event.listens_for(Session, "do_orm_execute")
def rerun_doc_hooks(state):
    if state.is_update and state.bind_mapper.class_ is AuditedDoc:
        state.session.info.setdefault("doc_log", []).append("bulk")

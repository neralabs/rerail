from sqlalchemy import event

from app.models import Note


@event.listens_for(Note, "after_update")
def record_note_update(mapper, connection, target):
    connection.info.setdefault("note_updates", []).append(target.id)

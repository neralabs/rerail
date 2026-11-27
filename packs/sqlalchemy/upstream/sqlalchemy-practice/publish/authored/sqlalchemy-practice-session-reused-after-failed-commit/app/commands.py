import click
from sqlalchemy.exc import SQLAlchemyError

from app.extensions import db
from app.models import Item


@click.command("rename-all")
def rename_all():
    # a CLI command is the outermost caller: its app context, and db.session with it, ends with the command
    try:
        for item in db.session.query(Item).all():
            item.name = item.name.strip()
        db.session.commit()
    except SQLAlchemyError as e:
        click.echo(f"failed: {e}")
        return


@click.command("rename-each")
def rename_each():
    for item in db.session.query(Item).all():
        try:
            item.name = item.name.strip()
            db.session.commit()
        except SQLAlchemyError as e:  # caught
            click.echo(f"failed: {e}")

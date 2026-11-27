from flask import Blueprint, flash, redirect, request
from slack_bolt import App
from sqlalchemy.exc import IntegrityError

from app.extensions import db
from app.models import Item

bp = Blueprint("items", __name__)
bolt_app = App()


def update_profile(user, data):
    # db.session is scoped to the request: the rest of the request runs on the failed session
    user.name = data["name"]
    try:
        db.session.commit()
    except IntegrityError:  # caught
        flash("That name is taken.")
    return redirect("/profile")


def create_item(name):
    try:
        db.session.add(Item(name=name))
        db.session.commit()
    except IntegrityError:
        db.session.rollback()
        flash("That name is taken.")
    return redirect("/items")


@bp.route("/items/<int:item_id>/rename", methods=["POST"])
def rename_item(item_id):
    # a route: Flask-SQLAlchemy removes db.session when the request ends, and nothing after the try uses it
    item = db.session.get(Item, item_id)
    item.name = request.form["name"]
    try:
        db.session.commit()
    except IntegrityError:
        flash("That name is taken.")
    return redirect("/items")


@bolt_app.action("archive-item")
def handle_archive(ack, body):
    # a Slack Bolt listener: nothing calls it but Bolt, and the app context ends with it
    ack()
    item = db.session.get(Item, int(body["value"]))
    item.archived = True
    try:
        db.session.commit()
    except IntegrityError:
        pass

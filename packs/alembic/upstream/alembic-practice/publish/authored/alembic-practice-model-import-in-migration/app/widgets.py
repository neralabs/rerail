from app.ext import db, plugins


class Widget(db.Model):
    """A Flask-SQLAlchemy model: its table name (`widget`) is generated."""

    id = db.Column(db.Integer, primary_key=True)
    color = db.Column(db.String(20))


class Gadget(plugins.Model):
    """A plugin descriptor that shares the attribute name, not a mapped class."""

    name = "gadget"

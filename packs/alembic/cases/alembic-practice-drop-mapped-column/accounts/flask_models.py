from flask_sqlalchemy import SQLAlchemy

db = SQLAlchemy()


class Widget(db.Model):
    id = db.Column(db.Integer, primary_key=True)
    color = db.Column(db.String(20))

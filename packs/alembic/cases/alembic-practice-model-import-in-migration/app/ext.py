from flask_sqlalchemy import SQLAlchemy

from app import registry

db = SQLAlchemy()
# A registry object of another library whose Model attribute is no declarative base.
plugins = registry.Registry()

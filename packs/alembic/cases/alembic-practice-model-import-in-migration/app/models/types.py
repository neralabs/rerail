import json

from sqlalchemy.types import CHAR, Text, TypeDecorator


class GUID(TypeDecorator):
    impl = CHAR
    cache_ok = True


class JSONDict(TypeDecorator):
    impl = Text
    cache_ok = True

    def process_bind_param(self, value, dialect):
        return json.dumps(value)

from pydantic import BaseModel

from app.mixins import TrimMixin


class Comment(TrimMixin, BaseModel):
    text: str
    author: str

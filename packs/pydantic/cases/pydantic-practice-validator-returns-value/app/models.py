from pydantic import BaseModel

from app.mixins import SlugMixin, TitleMixin


class Article(SlugMixin, TitleMixin, BaseModel):
    slug: str
    title: str

from .helpers import make_and_bump, touch, touch_any
from .models import Book


class Feed:
    async def receive(self, title):
        make_and_bump(title)  # caught

    async def receive_book(self, book: Book):
        touch(book)  # caught

    async def receive_any(self, obj):
        # Not reported: `obj.bump()` has an unknown target.
        touch_any(obj)

import asyncio

from asgiref.sync import sync_to_async
from django.db import transaction

from .models import Book


def total():
    return Book.objects.count()


def titles():
    out = []
    for b in Book.objects.filter(hits__gt=0):
        out.append(b.title)
    return out


def summary():
    return {"n": total()}


def archive(pk):
    with transaction.atomic():
        pass


def rename(pk, title):
    book = Book.objects.get(pk=pk)
    book.title = title
    book.save()


def touch(book: Book):
    book.bump()


def touch_any(obj):
    obj.bump()


def make_and_bump(title):
    b = Book(title=title)
    b.bump()


def fresh():
    b = Book(title="x")
    b.save()


def build_query():
    return Book.objects.filter(hits__gt=0)


def pure(x):
    return x * 2


def later():
    return lambda: Book.objects.count()


def defer():
    # The lambda calls total() later, not defer().
    return lambda: total()


@sync_to_async
def atotal():
    return Book.objects.count()


def via_offloaded():
    return atotal()


async def async_total():
    return await Book.objects.acount()


def makes_coroutine():
    return async_total()


def threaded():
    return sync_to_async(total)()

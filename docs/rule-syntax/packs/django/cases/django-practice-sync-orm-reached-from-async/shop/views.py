import asyncio

from asgiref.sync import sync_to_async

from . import helpers
from .helpers import archive, fresh, rename, summary, titles, total
from .models import Book


async def counts(request):
    n = total()  # caught
    t = titles()  # caught
    s = summary()  # caught
    archive(1)  # caught
    rename(1, "x")  # caught
    fresh()  # caught
    return n, t, s


async def by_module(request):
    return helpers.total()  # caught


async def safe(request):
    a = await sync_to_async(total)()
    b = await asyncio.to_thread(titles)
    c = await sync_to_async(lambda: summary())()
    loop = asyncio.get_running_loop()
    d = await loop.run_in_executor(None, total)
    e = await helpers.atotal()
    f = await helpers.via_offloaded()
    g = await helpers.makes_coroutine()
    h = await helpers.threaded()
    q = helpers.build_query()
    p = helpers.pure(2)
    later = helpers.later()
    deferred = helpers.defer()
    return a, b, c, d, e, f, g, h, q, p, later, deferred


async def direct(request):
    # Queries written in the async def itself.
    n = Book.objects.count()  # caught
    first = Book.objects.filter(hits__gt=0).first()  # caught
    for b in Book.objects.filter(hits__gt=0):  # caught
        n += b.hits
    async for b in Book.objects.filter(hits__gt=0):
        n += b.hits
    m = await Book.objects.acount()
    run = await sync_to_async(lambda: Book.objects.count())()
    return n, first, m, run


async def fetched(request):
    book = await Book.objects.aget(pk=1)
    book.bump()  # caught
    other = Book(title="x")
    other.bump()  # caught
    label = book.label()
    book.save()  # caught
    await book.asave()
    return label


async def nested(request):
    def inner():
        return total()

    return await sync_to_async(inner)()


def sync_view(request):
    return total()


async def unknown(request, fn):
    return fn()

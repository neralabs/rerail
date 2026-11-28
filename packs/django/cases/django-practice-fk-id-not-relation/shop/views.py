from shop import models as m
from shop.models import Book, BookInfo


def fetched(pk):
    book = Book.objects.get(pk=pk)
    print(book.author.id)  # caught
    latest = m.Book.objects.filter(title="x").order_by("pk").first()
    print(latest.cover.pk)  # caught
    for b in Book.objects.filter(title="x"):
        print(b.owner.pk)  # caught
    return book.author_id


def left_alone(book, pk):
    # A parameter: its class is not known.
    print(book.author.id)
    # Rebound to something that is not a row of Book.
    other = Book.objects.get(pk=pk)
    other = BookInfo(other.author)
    print(other.author.id)
    # Is it saved yet?
    row = Book.objects.get(pk=pk)
    if row.author.pk is None:
        pass
    # Assigned, not read.
    row.cover.pk = None
    # values() rows are dicts.
    for v in Book.objects.values("author"):
        print(v)
    # The loop variable is replaced in the body.
    for b in Book.objects.all():
        b = BookInfo(b.author)
        print(b.author.id)


def from_parameter(book):
    # A parameter replaced by a row: the row is what the name holds below.
    book = Book.objects.get(pk=book)
    return book.author.id  # caught


# The name is bound again in another way after the row is fetched: it may hold another object.
def loop_rebinds(pk, infos):
    book = Book.objects.get(pk=pk)
    print(book.title)
    for book in infos:
        print(book.author.id)


def with_rebinds(pk, opener):
    book = Book.objects.get(pk=pk)
    with opener() as book:
        print(book.author.id)


def unpack_rebinds(pk, pairs):
    book = Book.objects.get(pk=pk)
    book, other = pairs[0]
    print(book.author.id, other)


def comprehension_shadow(pk, infos):
    book = Book.objects.get(pk=pk)
    return [book.author.id for book in infos]


def loop_body_rebinds(infos):
    for b in Book.objects.all():
        for b in infos:
            print(b.author.id)

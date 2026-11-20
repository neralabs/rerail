from django.db import models

from shop.models import Book


def counts(author):
    a = len(Book.objects.filter(author=author))  # caught
    b = len((Book.objects.all().values_list("id", flat=True)))  # caught
    c = len(Book._default_manager.exclude(author=author))  # caught
    books = Book.objects.filter(author=author)
    d = len(books)
    e = Book.objects.filter(author=author).count()
    f = len([1, 2])
    g = len(Book.objects.get(pk=1).title)
    return a + b + c + d + e + f + g


def compared(author):
    # A real count compared with a number other than an emptiness bound.
    if len(Book.objects.filter(author=author)) > 1:  # caught
        pass
    if len(Book.objects.filter(author=author)) == 1:  # caught
        pass
    # A raw() queryset has no count().
    return len(Book.objects.raw("SELECT * FROM shop_book"))


def emptiness_tests(author):
    # Emptiness tests are django-practice-exists-for-existence's: the fix there is exists().
    if len(Book.objects.filter(author=author)) == 0:
        pass
    if 0 < len(Book.objects.filter(author=author)):
        pass
    if not len(Book.objects.filter(author=author)):
        pass
    if author and len(Book.objects.filter(author=author)):
        pass


def shadowed(len):
    return len(Book.objects.all())

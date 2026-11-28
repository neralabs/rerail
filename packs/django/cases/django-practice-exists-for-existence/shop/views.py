from django.db.models import Q

from shop.models import Book


def counted(author):
    if Book.objects.filter(author=author).count() > 0:  # caught
        pass
    if Book.objects.filter(author=author).count() == 0:  # caught
        pass
    if Book.objects.count() != 0:  # caught
        pass
    if Book.objects.filter(author=author).count() >= 1:  # caught
        pass
    if Book.objects.exclude(author=author).count() < 1:  # caught
        pass


def counted_reversed(author):
    if 0 < Book.objects.filter(author=author).count():  # caught
        pass
    if 1 > Book._default_manager.filter(author=author).count():  # caught
        pass
    has_books = (Book.objects.filter(author=author).count()) > 0  # caught
    if Book.objects.filter(author=author).count():  # caught
        pass
    while not Book.objects.filter(Q(author=author)).count():  # caught
        break
    return has_books


def loaded(author):
    if len(Book.objects.filter(author=author)) == 0:  # caught
        pass
    if len(Book.objects.filter(author=author)) != 0:  # caught
        pass
    if 0 == len(Book.objects.all()):  # caught
        pass


def loaded_tested(author):
    if not len(Book.objects.filter(author=author)):  # caught
        pass
    if author and len(Book.objects.filter(author=author)):  # caught
        pass


def truth(author):
    if Book.objects.filter(author=author):  # caught
        pass
    elif not Book.objects.exclude(author=author).order_by("title"):  # caught
        pass
    assert Book.objects.filter(author=author), "no books"  # caught
    found = bool(Book.objects.filter(author=author))  # caught
    label = "some" if Book.objects.filter(author=author) else "none"  # caught
    return found, label, [a for a in [author] if Book.objects.filter(author=a)]  # caught


def left_alone_counts(author):
    # A real count: the number matters.
    if Book.objects.filter(author=author).count() > 5:
        pass
    if Book.objects.filter(author=author).count() == 1:
        pass
    # A chained comparison is a range, not an emptiness test.
    if 0 < Book.objects.filter(author=author).count() < 10:
        pass
    # count() with arguments is not QuerySet.count().
    if "abc".count("a") > 0:
        pass


def left_alone_variable(author):
    # A queryset in a variable may be iterated after the test (the docs' "Don't overuse exists()").
    books = Book.objects.filter(author=author)
    if books:
        for b in books:
            print(b)
    if books.count() > 0:
        pass
    if len(books) == 0:
        pass
    # len() that is not an emptiness test belongs to django-practice-count-not-len.
    return len(Book.objects.filter(author=author))


def left_alone_other(author):
    # The manager itself is always true; that is a different bug.
    if Book.objects:
        pass
    # A raw() queryset has no exists().
    if Book.objects.raw("SELECT * FROM shop_book"):
        pass
    if len(Book.objects.raw("SELECT * FROM shop_book")) == 0:
        pass
    # Already right.
    if Book.objects.filter(author=author).exists():
        pass
    # A value, not a test: `or` returns the queryset.
    return Book.objects.filter(author=author) or Book.objects.all()

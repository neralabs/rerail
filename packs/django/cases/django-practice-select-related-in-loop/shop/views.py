from shop import models as m
from shop.models import Book


def related_managers():
    for book in Book.objects.filter(title="x"):  # caught
        print(book.tags.all())
    for book in Book.objects.all():  # caught
        print(book.review_set.count())
    return [b.tags.count() for b in Book.objects.order_by("title")]  # caught


def related_tests():
    for book in Book.objects.all():  # caught
        if book.tags.exists():
            pass
    # Prefetched: the related manager reads the cache.
    for book in Book.objects.prefetch_related("tags"):
        print(book.tags.all())


def foreign_keys():
    for book in Book.objects.filter(title="x"):  # caught
        print(book.author.name)
    for book in m.Book.objects.all():  # caught
        print(book.owner.email)
    titles = {b.author.name: b.title for b in Book.objects.all()}  # caught
    # Loaded by the query.
    for book in Book.objects.select_related("author"):
        print(book.author.name)
    # The key is on the row.
    for book in Book.objects.all():
        print(book.author_id, book.author.pk, book.author.id)
    return titles


def left_alone(books):
    # A queryset built elsewhere may be prefetched there.
    for book in books:
        print(book.tags.all(), book.author.name)
    # values() rows are dicts.
    for row in Book.objects.values("author"):
        print(row.items())
    # A JSON field, not a relation.
    for book in Book.objects.all():
        print(book.data.get("a"), book.title.upper())


def left_alone_more(model):
    # The model is not known: only related-manager calls are read.
    for obj in model.objects.all():
        print(obj.author.name)
    # A filtered or updated related manager queries each time, prefetched or not.
    for book in Book.objects.all():
        print(book.tags.filter(name="a"), book.tags.order_by("name").first())
        book.tags.all().update(name="b")
    # Code that runs later, not per row.
    for book in Book.objects.all():
        callbacks = [lambda: book.tags.all()]
    return callbacks


def properties():
    # Methods and properties of the model are not relations: prefetch_related('active_tags') raises.
    for book in Book.objects.all():
        print(book.active_tags.count(), list(book.recent_reviews.all()), book.owner_groups.exists())
    # A reverse relation the model does not define is a related manager.
    for book in Book.objects.all():  # caught
        print(book.review_set.exists())


def iterators():
    from shop.models import Review

    for review in Review.objects.all().iterator(chunk_size=500):  # caught
        print(review.book.title)
    for review in Review.objects.iterator():  # caught
        print(review.book.title)
    for book in Book.objects.filter(title="x").iterator(500):  # caught
        print(book.tags.all())
    # iterator() without chunk_size prefetches nothing (Django 5.0 raises with prefetch_related).
    for book in Book.objects.filter(title="x").iterator():
        print(book.tags.all())

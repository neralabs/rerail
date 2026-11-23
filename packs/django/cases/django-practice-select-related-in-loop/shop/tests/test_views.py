from shop.models import Book


def test_loop():
    for book in Book.objects.all():
        assert book.tags.all()

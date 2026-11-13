from shop.models import Book


def test_update():
    Book.objects.update(title="")

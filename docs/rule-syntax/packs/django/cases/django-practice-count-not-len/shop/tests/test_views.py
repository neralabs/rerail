from shop.models import Book


def test_counts():
    assert len(Book.objects.all()) == 2

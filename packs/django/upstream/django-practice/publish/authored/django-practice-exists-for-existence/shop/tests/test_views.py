from shop.models import Book


def test_empty():
    assert Book.objects.count() == 0
    assert not Book.objects.filter(title="x")

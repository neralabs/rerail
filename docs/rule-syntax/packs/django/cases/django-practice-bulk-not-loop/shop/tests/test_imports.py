from shop.models import Book


def test_many():
    for i in range(10):
        Book.objects.create(title=str(i))

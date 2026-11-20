# Test code (python/django dj_test_doc): shop/test.py.
from shop.models import Book


def book_count():
    return len(Book.objects.all())

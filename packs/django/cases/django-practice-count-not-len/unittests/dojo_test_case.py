# Test code (lib/django dj_test_doc): unittests/dojo_test_case.py.
from shop.models import Book


def book_count():
    return len(Book.objects.all())

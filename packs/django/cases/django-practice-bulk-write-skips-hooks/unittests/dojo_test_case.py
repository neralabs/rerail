from shop.models import Book


def reset():
    Book.objects.update(title="")

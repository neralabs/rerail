from catalog.models import Book


def apply_sort(items, sort):
    return items.order_by(sort)  # caught


def pick_fields(objects, fields):
    return objects.values(*fields)  # caught


def sort_books(books, key):
    return books.order_by(key)  # caught


def published():
    return Book.objects.filter(year__gte=1900)


def sort_rows(qs, key):
    # Every call passes a plain table, not a queryset: the name `qs` alone is no queryset.
    return qs.order_by(key)


def staff_sort(request):
    # Every call comes from a staff-only view.
    return Book.objects.order_by(request.GET["o"])


def shared_sort(request):
    # One call comes from a public view.
    return Book.objects.order_by(request.GET["o"])  # caught

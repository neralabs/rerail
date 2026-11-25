from django.http import JsonResponse

from catalog import helpers
from catalog.models import Author, Book
from catalog.views import Table


def by_helper(request):
    books = helpers.apply_sort(Book.objects.all(), request.GET["sort"])
    books = helpers.pick_fields(Book.objects.filter(year__gte=1900), request.GET.getlist("f"))
    qs = helpers.published()
    books = helpers.sort_books(qs, request.GET["sort"])
    rows = helpers.sort_rows(Table(), request.GET["sort"])
    return JsonResponse({"n": books.count(), "rows": rows})


def returned_queryset(request):
    books = helpers.published()
    return JsonResponse({"n": books.order_by(request.GET["sort"]).count()})  # caught


def related(request):
    author = Author.objects.get(pk=request.GET["author"])
    sort = request.GET.get("sort", "title")
    books = author.book_set.order_by(sort)  # caught
    shelves = author.shelf_set.values(sort)  # caught
    reviews = Book.objects.get(pk=1).reviews.order_by(sort)  # caught
    shelf_books = author.shelf_set.first().books.order_by(sort)  # caught
    return JsonResponse({"n": books.count() + shelves.count() + reviews.count() + shelf_books.count()})


def public_sort(request):
    return JsonResponse({"n": helpers.shared_sort(request).count()})


def substring_guard(request):
    books = helpers.published()
    orderby = request.GET.get("sort", "-year")
    # Only names holding the fixed text get through.
    if "year" in orderby:
        books = books.order_by(orderby)
    return JsonResponse({"n": books.count()})

from django.http import JsonResponse
from django.views import View

from catalog.models import Book


def sorted_by(request, sort):
    return JsonResponse({"n": Book.objects.order_by(sort).count()})  # caught


def by_position(request, position):
    # An `int` capture carries no field name.
    return JsonResponse({"n": Book.objects.order_by(position).count()})


class SortedView(View):
    def get(self, request, *args, **kwargs):
        books = Book.objects.order_by(self.kwargs["sort"])  # caught
        # The capture names are the URLconf's, not the client's.
        books = books.filter(**self.kwargs)
        return JsonResponse({"n": books.count()})

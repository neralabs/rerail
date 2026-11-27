from django.db import connection
from django.views import View
from django.views.generic import DetailView


def by_slug(request, slug):
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE slug = '{slug}'")  # caught


def by_pk(request, pk):
    # `<int:pk>`: the converter only passes digits.
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE id = {pk}")


def by_year(request, year, label):
    # `(?P<year>[0-9]{4})` carries digits; `(?P<label>[^/]+)` carries text.
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE year = {year}")
        cursor.execute(f"SELECT * FROM shop_product WHERE label = '{label}'")  # caught


class BaseReport(View):
    def rows(self):
        name = self.kwargs["name"]
        with connection.cursor() as cursor:
            cursor.execute(f"SELECT * FROM shop_report WHERE name = '{name}'")  # caught
            return cursor.fetchall()


class ReportView(BaseReport):
    def get(self, request, *args, **kwargs):
        return self.rows()


class ProductDetail(DetailView):
    # Routed with `<int:pk>` only: self.kwargs holds no text.
    def get_object(self, queryset=None):
        with connection.cursor() as cursor:
            cursor.execute(f"SELECT * FROM shop_product WHERE id = {self.kwargs['pk']}")
            return cursor.fetchone()


def unrouted(request, name):
    # No URLconf routes this view, so nothing says `name` comes from the URL.
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE name = '{name}'")

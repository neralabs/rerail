from django import forms
from django.db.models import F, Q
from django.db.models.functions import Extract, Trunc
from django.http import JsonResponse
from django.views import View
from django.views.generic import ListView

from catalog import queries
from catalog.models import Book

ALLOWED_FILTERS = {"title", "title__icontains", "year"}
ALLOWED_SORTS = {"title", "-title", "year", "-year"}
SORTS = {"title": "title", "newest": "-published"}


class SearchForm(forms.Form):
    title__icontains = forms.CharField(required=False)
    year = forms.IntegerField(required=False)


def search(request):
    books = Book.objects.filter(**request.GET.dict())  # caught
    books = books.exclude(**request.POST)  # caught
    return JsonResponse({"n": books.count()})


def search_comprehension(request):
    filters = {k: v for k, v in request.GET.items()}
    books = Book.objects.filter(**filters)  # caught
    counts = Book.objects.aggregate(**{name: F("year") for name in request.GET.getlist("as")})  # caught
    return JsonResponse({"n": books.count(), "counts": counts})


def search_field(request):
    field = request.GET.get("field", "title")
    value = request.GET.get("q", "")
    books = Book.objects.filter(**{f"{field}__icontains": value})  # caught
    lookups = {}
    lookups[field + "__startswith"] = value
    books = books.filter(Q(**lookups))  # caught
    extra = dict()
    extra.update(request.GET)
    books = books.filter(**extra)  # caught
    return JsonResponse({"n": books.count()})


def sorted_books(request):
    sort = request.GET.get("sort", "title")
    books = Book.objects.all().order_by(sort)  # caught
    books = books.values(request.GET["field"])  # caught
    books = Book.objects.only(*request.GET.getlist("fields"))  # caught
    books = Book.objects.order_by(*sort.split(","))  # caught
    books = Book.objects.annotate(v=F(request.GET["metric"]))  # caught
    books = Book.objects.annotate(p=Trunc("published", kind=request.GET["period"]))  # caught
    books = Book.objects.annotate(p=Extract("published", request.GET["part"]))  # caught
    return JsonResponse({"n": books.count()})


def through_helpers(request):
    books = queries.apply_filters(request.GET.dict())
    books = queries.apply_search(request.GET["field"], request.GET["q"])
    books = queries.apply_sort(Book.objects.all(), request.GET["sort"])
    books = queries.apply_known_sort(Book.objects.all(), request.GET["sort"])
    books = Book.objects.filter(**queries.query_filters(request))  # caught
    books = Book.objects.filter(**queries.fixed_filters(request))
    return JsonResponse({"n": books.count()})


def safe_search(request):
    q = request.GET.get("q", "")
    # Request values in keyword lookups are query parameters.
    books = Book.objects.filter(title__icontains=q, year=request.GET.get("year"))
    filters = {"title__icontains": q, "year": request.GET.get("year")}
    books = books.filter(**filters)
    books = books.filter(Q(**{"title__icontains": q}) | Q(author__name=q))
    # Keys kept to an allow-list.
    allowed = {k: v for k, v in request.GET.items() if k in ALLOWED_FILTERS}
    books = books.filter(**allowed)
    field = request.GET.get("field", "title")
    if field not in ALLOWED_FILTERS:
        field = "title"
    books = books.filter(**{f"{field}__icontains": q})
    # A form's cleaned data has the form's fixed field names.
    form = SearchForm(request.GET)
    if form.is_valid():
        books = books.filter(**form.cleaned_data)
    return JsonResponse({"n": books.count()})


def safe_sort(request):
    sort = request.GET.get("sort", "title")
    if sort not in ALLOWED_SORTS:
        sort = "title"
    books = Book.objects.order_by(sort)
    books = books.order_by(SORTS[request.GET.get("by", "title")])
    books = books.order_by(SORTS.get(request.GET.get("by"), "title"))
    if request.GET.get("o") in ALLOWED_SORTS:
        books = books.order_by(request.GET.get("o"))
    return JsonResponse({"n": books.count()})


def guarded_sort(request):
    ordering = request.GET.get("ordering", "")
    order_field = ordering.lstrip("-")
    books = Book.objects.all()
    # Each branch has checked the name it orders by.
    if order_field == "title":
        books = books.order_by(ordering)
    elif order_field in ("year", "author__name"):
        books = books.order_by(ordering)
    if ordering:
        books = books.order_by(ordering)  # caught
    return JsonResponse({"n": books.count()})


class BookList(ListView):
    def get_ordering(self):
        return self.request.GET.get("sort")

    def get_queryset(self):
        items = Book.objects.filter(year__gte=1900)
        items = items.order_by(self.request.GET.get("sort", "-published"))  # caught
        return items.order_by(self.get_ordering())  # caught


def by_relation(request):
    # get_field() raises for anything but one of the model's own field names.
    relation = request.GET.get("relation", "author")
    Book._meta.get_field(relation)
    books = Book.objects.filter(**{f"{relation}__in": request.GET.getlist("id")})
    return JsonResponse({"n": books.count()})


def not_a_queryset(request):
    table = Table()
    table.order_by(request.GET["sort"])
    rows = sorted(table.rows, key=lambda r: r[request.GET["sort"]])
    settings = {}
    settings.update(**request.GET.dict())
    return JsonResponse({"rows": rows, "settings": settings})


def create_book(request):
    # Mass assignment: the drf pack's rule, not this one.
    book = Book.objects.create(**request.POST.dict())
    return JsonResponse({"id": book.pk})


class BookDetail(View):
    def get(self, request, *args, **kwargs):
        # URLconf captures, not request data.
        book = Book.objects.get(**kwargs)
        return JsonResponse({"title": book.title})


class Table:
    rows = []

    def order_by(self, key):
        self.rows.sort(key=lambda r: r[key])

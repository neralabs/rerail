from catalog.models import Book


def apply_filters(params):
    return Book.objects.filter(**params)  # caught


def apply_search(field, term):
    return Book.objects.filter(**{f"{field}__icontains": term})  # caught


def apply_sort(qs, sort):
    return qs.order_by(sort)  # caught


def apply_known_sort(qs, sort):
    sort = clean_sort(sort)
    return qs.order_by(sort)


def clean_sort(sort):
    return sort if sort in {"title", "-title"} else "title"


def query_filters(request):
    return {key: value for key, value in request.GET.items()}


def fixed_filters(request):
    return {"title__icontains": request.GET.get("q", ""), "year": request.GET.get("year")}

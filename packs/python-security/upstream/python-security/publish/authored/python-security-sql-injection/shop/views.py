import logging

from django.db import connection, connections
from django.db.models.expressions import RawSQL
from django.http import JsonResponse

from shop import reports
from shop.models import Product

log = logging.getLogger(__name__)

SORTS = {"name": "p.name", "price": "p.price"}
ALLOWED_COLUMNS = {"name", "price", "created"}


def search(request):
    q = request.GET.get("q", "")
    rows = Product.objects.raw(f"SELECT * FROM shop_product WHERE name LIKE '%{q}%'")  # caught
    rows = Product.objects.raw("SELECT * FROM shop_product WHERE name = '%s'" % request.GET["name"])  # caught
    with connection.cursor() as cursor:
        cursor.execute("SELECT id FROM shop_product WHERE sku = '" + request.POST["sku"] + "'")  # caught
        cursor.execute("SELECT id FROM shop_product WHERE sku = %s", [request.POST["sku"]])
    return JsonResponse({"n": len(list(rows))})


def by_tags(request):
    tags = request.GET.getlist("tag")
    clause = " OR ".join(f"tag = '{t}'" for t in tags)
    cur = connections["replica"].cursor()
    cur.execute("SELECT id FROM shop_tag WHERE {}".format(clause))  # caught
    cur.executemany(f"UPDATE shop_tag SET hits = hits + 1 WHERE {clause}", [()])  # caught
    return JsonResponse({})


def legacy_filters(request):
    term = request.GET["term"]
    qs = Product.objects.extra(where=[f"name ILIKE '%%{term}%%'"])  # caught
    qs = qs.extra(select={"rank": "ts_rank(search, '" + term + "')"})  # caught
    qs = qs.annotate(score=RawSQL(f"SELECT count(*) FROM shop_review WHERE body ~ '{term}'", ()))  # caught
    return JsonResponse({"n": qs.count()})


def safe_search(request):
    q = request.GET.get("q", "")
    ids = request.GET.getlist("id")
    # Parameterised: the value travels as a parameter.
    rows = Product.objects.raw("SELECT * FROM shop_product WHERE name = %s", [q])
    # The placeholder list: only "%s" markers are spliced in, the values go in params.
    placeholders = ",".join(["%s"] * len(ids))
    rows = Product.objects.raw(f"SELECT * FROM shop_product WHERE id IN ({placeholders})", ids)
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE id IN ({','.join(['%s'] * len(ids))})", ids)
        # A number cannot carry SQL.
        limit = int(request.GET.get("limit", "10"))
        cursor.execute(f"SELECT * FROM shop_product LIMIT {limit}")
        cursor.execute("SELECT * FROM shop_product WHERE id = %d" % int(request.GET["id"]))
    qs = Product.objects.extra(where=["name = %s"], params=[q])
    qs = qs.annotate(score=RawSQL("SELECT count(*) FROM shop_review WHERE body ~ %s", (q,)))
    # The ORM parameterises lookups.
    qs = Product.objects.filter(name=request.GET["q"])
    # A request value in a log line never reaches a query.
    log.info(f"search for '{q}' from {request.META.get('REMOTE_ADDR')}")
    message = "SELECT * FROM shop_product WHERE name = '%s'" % q
    log.debug(message)
    return JsonResponse({"n": len(list(rows)) + qs.count()})


def sorted_products(request):
    # Identifiers picked through an allow-list mapping.
    col = SORTS[request.GET.get("sort", "name")]
    rows = Product.objects.raw(f"SELECT * FROM shop_product p ORDER BY {col}")
    # Identifiers checked against an allow-list before use.
    order = request.GET.get("order", "name")
    if order not in ALLOWED_COLUMNS:
        raise ValueError(order)
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product ORDER BY {order}")
    return JsonResponse({"n": len(list(rows))})


def sql_lab(request):
    # The client's own query, run as written: a deliberate feature, not an injection.
    with connection.cursor() as cursor:
        cursor.execute(request.POST["sql"])
        return JsonResponse({"rows": cursor.fetchall()})


def report(request):
    term = request.GET["q"]
    rows = reports.run(term)
    rows += reports.run_built(term)
    rows += reports.run_safe(term)
    return JsonResponse({"rows": rows})

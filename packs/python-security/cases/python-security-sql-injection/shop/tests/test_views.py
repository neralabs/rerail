from django.db import connection


def test_raw_lookup(request):
    q = request.GET["q"]
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE name = '{q}'")

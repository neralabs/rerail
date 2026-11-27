import re

from django.db import connection
from django.http import JsonResponse

NAME = re.compile(r"[a-z]+")
BLOCKED = {"admin"}


def validate_name(name):
    if not NAME.fullmatch(name):
        raise ValueError(name)


def check_exists(name):
    # Named like a check, but it runs the query itself (parameterised here): it decides nothing about the name.
    with connection.cursor() as cursor:
        cursor.execute("SELECT 1 FROM shop_product WHERE name = %s", [name])
        return cursor.fetchone() is not None


def run_spellcheck(name):
    return name.lower()


def validated(request):
    name = request.GET["name"]
    validate_name(name)
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE name = '{name}'")
    return JsonResponse({})


def exists_then_query(request):
    name = request.GET["name"]
    if not check_exists(name):
        return JsonResponse({}, status=404)
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE name = '{name}'")  # caught
    return JsonResponse({})


def substring_is_no_check(request):
    name = request.GET["name"]
    run_spellcheck(name)
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE name = '{name}'")  # caught
    return JsonResponse({})


def blocklist_is_no_check(request):
    name = request.GET["name"]
    if name in BLOCKED:
        raise ValueError(name)
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM shop_product WHERE name = '{name}'")  # caught
    return JsonResponse({})

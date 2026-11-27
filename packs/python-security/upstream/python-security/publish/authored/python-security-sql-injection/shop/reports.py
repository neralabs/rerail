from django.db import connection

from shop.sqlbuild import build_where


def run(term):
    sql = f"SELECT * FROM shop_report WHERE author = '{term}'"
    with connection.cursor() as cursor:
        cursor.execute(sql)  # caught
        return cursor.fetchall()


def run_built(term):
    sql = build_where(term)
    cursor = connection.cursor()
    cursor.execute(sql)  # caught
    return cursor.fetchall()


def run_safe(term):
    with connection.cursor() as cursor:
        cursor.execute("SELECT * FROM shop_report WHERE author = %s", [term])
        return cursor.fetchall()


def nightly():
    # Server-side values only: nothing from a request.
    table = "shop_report"
    with connection.cursor() as cursor:
        cursor.execute(f"DELETE FROM {table} WHERE created < now() - interval '90 days'")

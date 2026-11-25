from django import forms
from django.db import connection
from rest_framework import generics, serializers


class LimitSerializer(serializers.Serializer):
    limit = serializers.IntegerField(max_value=100)
    direction = serializers.ChoiceField(choices=["ASC", "DESC"])
    since = serializers.DateField()
    owner = serializers.UUIDField()
    name = serializers.CharField()


class PagedSerializer(LimitSerializer):
    # Inherits limit/direction/since/owner/name from the base serializer of the run.
    page = serializers.IntegerField()


class LimitForm(forms.Form):
    limit = forms.IntegerField()
    order = forms.TypedChoiceField(choices=[("asc", "asc"), ("desc", "desc")])
    term = forms.CharField()


def serializer_typed(request):
    s = LimitSerializer(data=request.data)
    s.is_valid(raise_exception=True)
    n = s.validated_data["limit"]
    d = s.validated_data["direction"]
    with connection.cursor() as cursor:
        # Validated int and fixed choice: no client text can reach the SQL.
        cursor.execute(f"SELECT * FROM t ORDER BY id {d} LIMIT {n}")
        cursor.execute(f"SELECT * FROM t WHERE since > '{s.validated_data.get('since')}'")
        cursor.execute(f"SELECT * FROM t WHERE owner = '{s.validated_data['owner']}'")
        # A CharField is free text.
        cursor.execute(f"SELECT * FROM t WHERE name = '{s.validated_data['name']}'")  # caught


def subclass_typed(request):
    s = PagedSerializer(data=request.data)
    s.is_valid(raise_exception=True)
    with connection.cursor() as cursor:
        cursor.execute(f"SELECT * FROM t LIMIT {s.validated_data['limit']} OFFSET {s.validated_data['page']}")
        cursor.execute("SELECT * FROM t WHERE name = '%s'" % s.validated_data["name"])  # caught


def form_typed(request):
    form = LimitForm(request.GET)
    if form.is_valid():
        n = form.cleaned_data["limit"]
        order = form.cleaned_data["order"]
        with connection.cursor() as cursor:
            cursor.execute("SELECT * FROM t ORDER BY id %s LIMIT %d" % (order, n))
            cursor.execute("SELECT * FROM t WHERE term = '%s'" % form.cleaned_data["term"])  # caught


class ReportList(generics.GenericAPIView):
    serializer_class = LimitSerializer

    def post(self, request):
        serializer = self.get_serializer(data=request.data)
        serializer.is_valid(raise_exception=True)
        with connection.cursor() as cursor:
            cursor.execute(f"SELECT * FROM t LIMIT {serializer.validated_data['limit']}")
            cursor.execute(f"SELECT * FROM t WHERE name = '{serializer.validated_data['name']}'")  # caught

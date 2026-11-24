import ast
import importlib
import json
import marshal
import pickle
import shelve

import yaml
from django.core import signing
from django.http import HttpResponse, JsonResponse
from django.template import Context, Engine, Template, engines

from app import codec

ALLOWED_FORMATS = {"csv", "xlsx"}


def calculate(request):
    expr = request.GET.get("expr", "")
    result = eval(expr)  # caught
    return JsonResponse({"result": result})


def run_snippet(request):
    code = request.POST["code"]
    exec(compile(code, "<snippet>", "exec"))  # caught
    return HttpResponse("ok")


def export(request):
    fmt = request.GET["format"]
    exporter = importlib.import_module(f"app.exporters.{fmt}")  # caught
    return exporter.render()


def export_checked(request):
    fmt = request.GET["format"]
    if fmt not in ALLOWED_FORMATS:
        return HttpResponse(status=400)
    exporter = importlib.import_module(f"app.exporters.{fmt}")
    return exporter.render()


def plugin(request):
    mod = __import__(request.GET["plugin"])  # caught
    return HttpResponse(mod.__name__)


def restore(request):
    state = pickle.loads(request.body)  # caught
    return JsonResponse(state)


def restore_file(request):
    upload = request.FILES["state"]
    state = pickle.load(upload)  # caught
    data = upload.read()
    obj = pickle.loads(data)  # caught
    code = marshal.loads(data)  # caught
    return JsonResponse({"state": state, "obj": obj, "code": str(code)})


def open_shelf(request):
    with shelve.open(request.GET["db"]) as db:  # caught
        return JsonResponse(dict(db))


def import_config(request):
    raw = request.body.decode()
    a = yaml.load(raw)  # caught
    b = yaml.load(raw, Loader=yaml.FullLoader)  # caught
    c = yaml.load(raw, yaml.UnsafeLoader)  # caught
    d = yaml.unsafe_load(raw)  # caught
    e = yaml.full_load(raw)  # caught
    f = list(yaml.load_all(raw, Loader=yaml.Loader))  # caught
    return JsonResponse({"a": a, "b": b, "c": c, "d": d, "e": e, "f": f})


def import_config_safe(request):
    raw = request.body.decode()
    a = yaml.safe_load(raw)
    b = yaml.load(raw, Loader=yaml.SafeLoader)
    c = yaml.load(raw, Loader=yaml.CSafeLoader)
    d = yaml.load(raw, yaml.BaseLoader)
    e = yaml.load(raw, Loader=getattr(yaml, "CSafeLoader", yaml.SafeLoader))
    f = json.loads(raw)
    g = ast.literal_eval(raw)
    h = signing.loads(request.GET["token"])
    i = eval("1 + 2")
    return JsonResponse({"a": a, "b": b, "c": c, "d": d, "e": e, "f": f, "g": g, "h": h, "i": i})


def preview(request):
    body = request.POST["body"]
    tpl = Template("<h1>Preview</h1>" + body)  # caught
    page = Engine.get_default().from_string(body)  # caught
    other = engines["django"].from_string(body)  # caught
    return HttpResponse(tpl.render(Context({})) + page.render(Context({})) + other.render({}))


def greet(request):
    tpl = Template("Hello {{ name }}")
    return HttpResponse(tpl.render(Context({"name": request.GET["name"]})))


def load_blob(request):
    obj = codec.decode(request.body)
    return JsonResponse(obj)


def load_cached(request, key):
    return JsonResponse(codec.from_cache(key))

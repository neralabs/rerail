from django.http import HttpResponse
from django.template import Context, Engine, Template, engines

ENGINE = Engine(debug=False)


def preview(request):
    t = Template(request.POST["tpl"])  # caught
    return HttpResponse(t.render(Context({"user": request.user})))


def preview_engine(request):
    t = ENGINE.from_string(request.POST["tpl"])  # caught
    t2 = Engine.get_default().from_string(request.POST["tpl"])  # caught
    t3 = engines["django"].from_string(request.POST["tpl"])  # caught
    return HttpResponse(t.render(Context({})) + t2.render(Context({})) + t3.render({}))


def fixed(request):
    t = Template("<p>{{ name }}</p>")
    return HttpResponse(t.render(Context({"name": request.POST["name"]})))

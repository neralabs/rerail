import cloudpickle
import jinja2
from flask import Flask, current_app, render_template_string, request
from jinja2 import Environment, Template
from mako.template import Template as MakoTemplate

from app.notify import Notifier

app = Flask(__name__)
env = Environment(autoescape=True)


@app.route("/rpc", methods=["POST"])
def rpc():
    fn = cloudpickle.loads(request.get_data())  # caught
    return str(fn())


@app.route("/hello")
def hello():
    name = request.args.get("name", "")
    page = render_template_string("<p>Hello " + name + "</p>")  # caught
    safe = render_template_string("<p>Hello {{ name }}</p>", name=name)
    return page + safe


@app.route("/notify", methods=["POST"])
def notify():
    body = request.form["body"]
    a = Template(body).render()  # caught
    b = env.from_string(body).render()  # caught
    c = jinja2.Environment().from_string(body).render()  # caught
    d = current_app.jinja_env.from_string(body).render()  # caught
    e = MakoTemplate(body).render()  # caught
    f = MakoTemplate(filename="/srv/templates/notify.txt").render(body=body)
    g = Notifier().render(body)
    return a + b + c + d + e + f + g


@app.route("/page/<slug>")
def page(slug):
    return render_template_string(slug)  # caught


@app.route("/page/<int:n>")
def page_number(n):
    return render_template_string(str(n))

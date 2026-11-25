import os

from flask import Blueprint, send_file

from webapp import files

bp = Blueprint("pages", __name__)
PAGES = "/srv/pages"


@bp.route("/page/<string:slug>")
def page(slug):
    return send_file(os.path.join(PAGES, f"{slug}.html"))


@bp.route("/page-raw/<path:slug>")
def page_raw(slug):
    return send_file(os.path.join(PAGES, f"{slug}.html"))  # caught


@bp.route("/page-whole/<slug>")
def page_whole(slug):
    return send_file(os.path.join(PAGES, slug))  # caught


@bp.route("/doc/<slug>")
def doc(slug):
    return files.read_note(slug)


@bp.route("/doc-any/<path:name>")
def doc_any(name):
    return files.read_note(name)

import os

from flask import Flask, abort, request, send_file, send_from_directory
from flask_uploads import UploadSet
from werkzeug.security import safe_join
from werkzeug.utils import secure_filename

from webapp import files

app = Flask(__name__)
UPLOADS = "/srv/uploads"
STATIC = "/srv/static"
photos = UploadSet("photos")


@app.route("/download")
def download():
    return send_file(os.path.join(UPLOADS, request.args["f"]))  # caught


@app.route("/static/<group>/<path:filename>")
def group_static(group, filename):
    return send_from_directory(f"{STATIC}/{group}", path=filename)  # caught


@app.route("/upload", methods=["POST"])
def upload():
    f = request.files["file"]
    f.save(os.path.join(UPLOADS, f.filename))  # caught
    return "ok"


@app.route("/photo", methods=["POST"])
def photo():
    photos.save(request.files["photo"], name=request.form["name"])  # caught
    return "ok"


@app.route("/notes/<name>")
def note(name):
    return files.read_note(name)


@app.route("/files/<path:filename>")
def serve(filename):
    return send_from_directory(STATIC, filename)


@app.route("/upload-safe", methods=["POST"])
def upload_safe():
    f = request.files["file"]
    f.save(os.path.join(UPLOADS, secure_filename(f.filename)))
    return "ok"


@app.route("/avatar/<filename>")
def avatar(filename):
    filename = secure_filename(filename)
    return send_file(os.path.join(UPLOADS, "avatars", filename))


@app.route("/report")
def report():
    path = safe_join(UPLOADS, request.args["name"])
    if path is None:
        abort(404)
    return send_file(path)


@app.route("/export/<int:export_id>")
def export(export_id):
    return send_file(os.path.join(UPLOADS, f"{export_id}.csv"))

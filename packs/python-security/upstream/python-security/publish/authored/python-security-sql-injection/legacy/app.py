import sqlite3

from flask import Flask, request

app = Flask(__name__)


@app.route("/notes")
def notes():
    db = sqlite3.connect("notes.db")
    cur = db.cursor()
    cur.execute("SELECT body FROM notes WHERE owner = '%s'" % request.args["owner"])  # caught
    cur.execute("SELECT body FROM notes WHERE owner = ?", (request.args["owner"],))
    return {"notes": cur.fetchall()}


@app.route("/purge", methods=["POST"])
def purge():
    db = sqlite3.connect("notes.db")
    tag = request.form["tag"]
    db.execute(f"DELETE FROM notes WHERE tag = '{tag}'")  # caught
    db.commit()
    return {}

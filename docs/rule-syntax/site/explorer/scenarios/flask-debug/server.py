import os

from flask import Flask

app = Flask(__name__)


@app.route("/")
def index():
    return "hello"


if __name__ == "__main__":
    app.run(debug=True)


def run_from_env():
    app.run(debug=os.environ.get("FLASK_DEBUG") == "1")


def run_quiet():
    app.run(debug=False)

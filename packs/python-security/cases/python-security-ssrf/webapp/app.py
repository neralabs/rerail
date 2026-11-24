from urllib.parse import urljoin
from urllib.request import urlopen

import requests
from flask import Flask, jsonify, request

app = Flask(__name__)
API_BASE = "https://api.example.com/v2/"


# ---- reported ----------------------------------------------------------------------------------------------------


@app.route("/fetch")
def fetch():
    return urlopen(request.args["url"], timeout=5).read()  # caught


@app.post("/webhook/test")
def webhook_test():
    payload = request.get_json()
    hook = payload.get("hook_url") or app.config["DEFAULT_HOOK"]
    requests.post(hook, json={"test": True}, timeout=5)  # caught
    return jsonify(ok=True)


@app.route("/relay/<path:target>")
def relay(target):
    return requests.get(urljoin(API_BASE, target), timeout=5).text  # caught


@app.route("/hosts/<host>")
def host_status(host):
    return requests.get(f"https://{host}.internal.example.com/status", timeout=5).text  # caught


# ---- left alone --------------------------------------------------------------------------------------------------


@app.route("/items/<int:item_id>")
def item(item_id):
    return requests.get(f"https://api.example.com/items/{item_id}", timeout=5).text


@app.route("/docs/<name>")
def doc(name):
    return requests.get(urljoin(API_BASE, f"docs/{name}"), timeout=5).text


@app.route("/lookup")
def lookup():
    return requests.get(API_BASE + "lookup", params={"q": request.args.get("q")}, timeout=5).text

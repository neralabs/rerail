"""Run by hand against a local server: a hang here is in front of whoever ran it."""
import requests

BASE_URL = "https://127.0.0.1:5000"

session = requests.Session()
session.headers.update({"Authorization": "Bearer CHANGE_ME"})

res = session.post(f"{BASE_URL}/api/records")
assert res.status_code == 201

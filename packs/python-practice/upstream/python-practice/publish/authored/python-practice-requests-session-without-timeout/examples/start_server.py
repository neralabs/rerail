"""Example: start a server through the REST API, run by hand against a local hub."""
import requests


def make_session(token):
    session = requests.Session()
    session.headers = {"Authorization": f"token {token}"}
    return session


def main():
    session = make_session("CHANGE_ME")
    r = session.get("http://127.0.0.1:8000/hub/api/status")
    r.raise_for_status()

import subprocess

from flask import Flask, request

app = Flask(__name__)


@app.route("/whois")
def whois():
    return subprocess.getoutput("whois " + request.args["q"])  # caught


@app.route("/run", methods=["POST"])
def run():
    return subprocess.check_output(request.form["cmd"].split())  # caught


@app.route("/logs/<name>")
def logs(name):
    status, out = subprocess.getstatusoutput(f"tail -n 50 /var/log/{name}.log")  # caught
    return out


@app.route("/jobs/<int:job_id>")
def job(job_id):
    return subprocess.getoutput(f"jobctl show {job_id}")

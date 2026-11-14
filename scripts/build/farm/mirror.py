"""The farm's live mirror (README.md, "Live mirror"): each host keeps the files its compiles read on
its own disk, at the build machine's paths, and gets only what changed, over one connection.

A daemon here (`serve`) holds a `farm-agent` per host over ssh. A compile that goes to a host asks
the daemon (`compile`, over a Unix socket): the daemon compares the files the compile may read
with what it has sent that host, sends the difference, then the compile, on the same ordered
stream. The agent writes the files before it reads the compile, so the compile sees them; its
outputs come back on the stream. One scan of a directory serves every compile that starts during
it. Every file the compile read (its dep-info) must be as it is here, or the caller compiles here.
"""
import json
import os
import socket
import struct
import subprocess
import sys
import threading
import time

import farm

SOCKET = farm.state("mirror.sock")
IDLE_SECONDS = 15 * 60
# How long a scan of a checkout's file list (git ls-files) is reused; a file added since is caught by
# the read check, and the compile then runs here.
LIST_SECONDS = 10


def send(f, header, payload=b""):
    data = json.dumps(dict(header, size=len(payload))).encode()
    f.write(struct.pack(">I", len(data)) + data + payload)
    f.flush()


def receive(f):
    head = f.read(4)
    if len(head) < 4:
        return None, None
    header = json.loads(f.read(struct.unpack(">I", head)[0]))
    size = header.get("size", 0)
    payload = f.read(size) if size else b""
    return header, payload


def stat(path):
    try:
        st = os.stat(path)
        return st.st_mtime_ns, st.st_size, st.st_mode
    except OSError:
        return None


class Host:
    """One host's agent: the files it has, and the compiles waiting on it."""

    def __init__(self, name):
        self.name = name
        farm.CHANNEL[0] = "mirror"
        self.proc = subprocess.Popen(farm.ssh(name, os.path.join(farm.SHARE, "farm-agent")),
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.lock = threading.Lock()
        self.sent = {}
        self.waiting = {}
        self.next_id = 0
        threading.Thread(target=self.read, daemon=True).start()

    def alive(self):
        return self.proc.poll() is None

    def ask(self, header):
        """Sends `header` with an id and waits for the agent's reply to it."""
        done = threading.Event()
        with self.lock:
            self.next_id += 1
            job_id = self.next_id
            self.waiting[job_id] = {"event": done, "result": None}
            send(self.proc.stdin, dict(header, id=job_id))
        done.wait(farm.COMPILE_SECONDS)
        return self.waiting.pop(job_id, {}).get("result")

    def learn(self, paths):
        """What the host already has of `paths` (from an earlier connection), asked once a path."""
        unknown = [p for p in paths if p not in self.sent]
        if unknown:
            reply = self.ask({"op": "have", "paths": unknown})
            for p, seen in zip(unknown, (reply or {}).get("seen") or []):
                self.sent[p] = seen

    def put(self, path, st):
        if self.sent.get(path) == list(st[:2]):
            return
        try:
            data = open(path, "rb").read()
        except OSError:
            return
        with self.lock:
            send(self.proc.stdin, {"op": "put", "path": path, "mtime_ns": st[0], "mode": st[2]}, data)
            self.sent[path] = list(st[:2])

    def run(self, job):
        return self.ask(dict(job, op="run"))

    def read(self):
        while True:
            try:
                header, payload = receive(self.proc.stdout)
            except (OSError, ValueError):
                header = None
            if header is None:
                for waiting in self.waiting.values():
                    waiting["event"].set()
                return
            if header["op"] == "file":
                path = header["path"]
                tmp = f"{path}.farm-{os.getpid()}"
                with open(tmp, "wb") as f:
                    f.write(payload)
                os.replace(tmp, path)
            elif header["op"] in ("done", "had"):
                waiting = self.waiting.get(header["id"])
                if waiting:
                    waiting["result"] = header
                    waiting["event"].set()


class Scans:
    """Directory scans, shared: a scan that began after a compile began serves it."""

    def __init__(self):
        self.lock = threading.Lock()
        self.done = {}
        self.locks = {}

    def files(self, key, since, make):
        with self.lock:
            lock = self.locks.setdefault(key, threading.Lock())
        with lock:
            began, files = self.done.get(key, (0, None))
            if files is not None and began >= since:
                return files
            began = time.time()
            files = make()
            self.done[key] = (began, files)
            return files


hosts = {}
hosts_lock = threading.Lock()
scans = Scans()
lists = {}
last_job = [time.time()]


def host(name):
    with hosts_lock:
        h = hosts.get(name)
        if h is None or not h.alive():
            h = hosts[name] = Host(name)
        return h


def checkout_files(checkout):
    began, names = lists.get(checkout, (0, None))
    if names is None or time.time() - began > LIST_SECONDS:
        names = [os.path.join(checkout, n) for n in farm.tracked(checkout)]
        lists[checkout] = (time.time(), names)
    return names


def tree(root):
    return [os.path.join(d, f) for d, _, fs in os.walk(root) for f in fs]


def top(root, test):
    try:
        return [os.path.join(root, f) for f in os.listdir(root) if test(f)]
    except OSError:
        return []


def needed(job, since):
    """The files `job` may read, with their stats here: (path, (mtime_ns, size, mode))."""
    out = []
    for root, kind in job["roots"]:
        if kind == "checkout":
            make = lambda root=root: [(p, stat(p)) for p in checkout_files(root)]
        elif kind == "tree":
            make = lambda root=root: [(p, stat(p)) for p in tree(root)]
        elif kind == "deps":
            make = lambda root=root: [(p, stat(p)) for p in top(root, lambda f: f.endswith(".rmeta") or (f.startswith("lib") and f.endswith(".dylib")))]
        else:
            make = lambda root=root: [(p, stat(p)) for p in top(root, lambda f: True)]
        out += scans.files((root, kind), since, make)
    out += [(p, stat(p)) for p in job["files"]]
    return [(p, st) for p, st in out if st is not None]


def handle(conn):
    f = conn.makefile("rwb")
    try:
        job, _ = receive(f)
        if job is None:
            return
        last_job[0] = time.time()
        h = host(job["host"])
        started = time.time()
        files = needed(job, job["since"])
        h.learn([p for p, _ in files])
        for path, st in files:
            h.put(path, st)
        sent = time.time()
        result = h.run(job["run"]) if h.alive() else None
        reply = result or {"code": -1, "stdout": "", "stderr": "", "read": {"?": None}, "why": "the agent stopped"}
        reply["times"] = {"sync": round(sent - started, 3), "compile": round(time.time() - sent, 3), "files": len(files)}
        send(f, reply)
    finally:
        f.close()
        conn.close()


def serve():
    """The daemon: one per user; exits after IDLE_SECONDS without a compile."""
    lock = farm.Lock(SOCKET + ".lock", blocking=False)
    if not lock.held:
        return
    try:
        os.unlink(SOCKET)
    except OSError:
        pass
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(SOCKET)
    server.listen(256)
    server.settimeout(30)
    while time.time() - last_job[0] < IDLE_SECONDS:
        try:
            conn, _ = server.accept()
        except socket.timeout:
            continue
        threading.Thread(target=handle, args=(conn,), daemon=True).start()
    server.close()
    os.unlink(SOCKET)


def connect():
    for attempt in range(60):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            s.connect(SOCKET)
            return s
        except OSError:
            s.close()
        if attempt == 0:
            subprocess.Popen(["nice", "-n", "5", sys.executable, os.path.join(farm.SHARE, "mirror.py")], start_new_session=True,
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(0.05)
    return None


def compile(job):
    """Asks the daemon to run `job` on its host; the daemon's reply, or None."""
    s = connect()
    if s is None:
        return None
    f = s.makefile("rwb")
    try:
        send(f, job)
        reply, _ = receive(f)
        return reply
    finally:
        f.close()
        s.close()


if __name__ == "__main__":
    serve()

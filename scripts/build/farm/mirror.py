"""The farm's live mirror (README.md, "Live mirror"): each host keeps the files its compiles read on
its own disk, at the build machine's paths, and gets only what changed, over one connection.

A daemon here (`serve`) holds a `farm-agent` per host over ssh. A compile that goes to a host asks
the daemon (`compile`, over a Unix socket): the daemon compares the files the compile may read
with what it has sent that host, sends the difference, then the compile, on the same ordered
stream. The agent writes the files before it reads the compile, so the compile sees them; its
outputs come back on the stream. One scan of a directory serves every compile that starts during
it. Every file the compile read (its dep-info) must be as it is here, or the caller compiles here.
"""
import hmac
import json
import os
import secrets
import select
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
        self.proc = subprocess.Popen(farm.ssh(name, farm.on_host("farm-agent"), "mirror"),
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=open(farm.state("mirror", f"{name}.err"), "ab"))
        self.w, self.r = self.proc.stdin, self.proc.stdout
        self.transport = "ssh"
        if farm.config().get("tcp", True) and os.environ.get("RERAIL_FARM_TCP", "1") != "0":
            self.over_tcp()
        self.lock = threading.Lock()
        self.sent = {}
        self.waiting = {}
        self.next_id = 0
        self.closed = False
        threading.Thread(target=self.read, daemon=True).start()

    def over_tcp(self):
        """Moves the stream off ssh onto a plain TCP connection the agent opens back to here, over
        the fast link (farm-agent). ssh then carries nothing but the lifeline. On any failure the
        stream stays on ssh."""
        listener = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
        try:
            listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
            listener.bind(("::", 0))
            listener.listen(1)
            token = secrets.token_hex(16)
            send(self.w, {"op": "tcp", "port": listener.getsockname()[1], "token": token})
            deadline = time.time() + 8
            while time.time() < deadline:
                ready, _, _ = select.select([listener, self.r], [], [], deadline - time.time())
                if self.r in ready:
                    header, _ = receive(self.r)  # the agent could not connect, or stopped
                    farm.log(f"{self.name}: mirror over ssh: {(header or {}).get('why', 'the agent stopped')}")
                    return
                if listener in ready:
                    conn, peer = listener.accept()
                    conn.settimeout(5)
                    line = conn.makefile("rb").readline(64).strip()
                    if not hmac.compare_digest(line, token.encode()):
                        conn.close()  # not the agent: keep waiting for it
                        continue
                    conn.settimeout(None)
                    conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    for opt in (socket.SO_SNDBUF, socket.SO_RCVBUF):
                        conn.setsockopt(socket.SOL_SOCKET, opt, 8 << 20)
                    conn.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
                    # A dead link is noticed in about 10 s, not macOS's default 2 hours.
                    # (TCP_KEEPALIVE, the idle time, is 0x10 on macOS; Python names it only on some builds.)
                    for opt, v in ((getattr(socket, "TCP_KEEPALIVE", 0x10), 4), (socket.TCP_KEEPINTVL, 2), (socket.TCP_KEEPCNT, 3)):
                        conn.setsockopt(socket.IPPROTO_TCP, opt, v)
                    self.sock = conn
                    self.w, self.r = conn.makefile("wb"), conn.makefile("rb")
                    self.transport = "tcp"
                    farm.log(f"{self.name}: mirror over tcp")
                    return
            farm.log(f"{self.name}: mirror over ssh: the agent did not connect back")
        except OSError as e:
            farm.log(f"{self.name}: mirror over ssh: {e!r}")
        finally:
            listener.close()

    def alive(self):
        return not self.closed and self.proc.poll() is None

    def ask(self, header, timeout=farm.COMPILE_SECONDS):
        """Sends `header` with an id and waits for the agent's reply to it (None: none came)."""
        done = threading.Event()
        with self.lock:
            if self.closed:
                return None
            self.next_id += 1
            job_id = self.next_id
            self.waiting[job_id] = {"event": done, "result": None}
            send(self.w, dict(header, id=job_id))
        done.wait(timeout)
        return self.waiting.pop(job_id, {}).get("result")

    def stop(self):
        with self.lock:
            self.closed = True
        try:
            if self.transport == "tcp":
                self.sock.close()
            self.proc.kill()
        except OSError:
            pass

    def learn(self, paths):
        """What the host already has of `paths` (from an earlier connection), asked once a path."""
        unknown = [p for p in paths if p not in self.sent]
        if unknown:
            reply = self.ask({"op": "have", "paths": unknown}, timeout=60)
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
            # Another compile's scan may have sent it meanwhile.
            if self.sent.get(path) == list(st[:2]):
                return
            send(self.w, {"op": "put", "path": path, "mtime_ns": st[0], "mode": st[2]}, data)
            self.sent[path] = list(st[:2])

    def run(self, job):
        return self.ask(dict(job, op="run"))

    def read(self):
        while True:
            try:
                header, payload = receive(self.r)
            except (OSError, ValueError):
                header = None
            if header is None:
                with self.lock:
                    self.closed = True
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


def drop(name, h, why=None):
    """Stops `h`, `name`'s agent; with `why`, the host is down for its back-off (`farm.mark_down`)."""
    with hosts_lock:
        if hosts.get(name) is h:
            del hosts[name]
    h.stop()
    if why:
        farm.mark_down(name, why)


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
        send(f, work(job))
    except Exception as e:
        try:
            send(f, {"code": -1, "stdout": "", "stderr": "", "read": {"?": None}, "why": f"the daemon failed: {e!r}"})
        except OSError:
            pass
    finally:
        f.close()
        conn.close()


def failed(why):
    return {"code": -1, "stdout": "", "stderr": "", "read": {"?": None}, "why": why}


def work(job):
    """Sends what `job` may read, then runs it. What fails here never fails twice the same way: a
    host whose agent stopped or could not write is down for its back-off, and one whose run failed
    or read other files than this machine's is asked again what it holds (it may have been cleaned)."""
    name = job["host"]
    known = next((x for x in farm.hosts() if x["name"] == name), {"name": name})
    health = farm.health(known)
    if not health.get("ok"):
        return failed(f"{name} is not usable: {health.get('why')}")
    h = host(name)
    started = time.time()
    try:
        files = needed(job, job["since"])
        if job.get("verify"):
            for p, _ in files:
                h.sent.pop(p, None)
        h.learn([p for p, _ in files])
        for path, st in files:
            h.put(path, st)
    except OSError as e:
        drop(name, h, f"the mirror's connection broke: {e!r}")
        return failed(f"the connection to {name} broke while sending")
    sent = time.time()
    result = h.run(job["run"]) if h.alive() else None
    if result is None:
        drop(name, h, "its mirror agent stopped, or did not answer")
        reply = failed("the agent stopped")
    elif result["code"] == 70:
        drop(name, h, f"a file could not be written there (its disk full, or not writable): {result.get('stderr', '')[-200:]}")
        reply = result
    else:
        reply = result
        cwd = job["run"].get("cwd", "/")
        stale = [p for p, seen in result.get("read", {}).items()
                 if seen is None or list(stat(os.path.join(cwd, p)) or [])[:2] != seen]
        if result["code"] != 0 or stale:
            h.sent.clear()
    reply["times"] = {"sync": round(sent - started, 3), "compile": round(time.time() - sent, 3), "files": len(files),
                      "transport": h.transport}
    return reply


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

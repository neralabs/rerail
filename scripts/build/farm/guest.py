"""The farm's guests: hosts that are virtual machines on another machine (README, "Guests") — a
Linux VM on a Mac (Lima, OrbStack), a Windows VM on a Mac (Parallels), WSL on a Windows PC. A
guest takes its machine's memory while it runs, and a VM gives little of it back while it idles,
so a guest runs only while the farm uses it:

  - started when a command needs it (`test-remote`, when no running host of its OS has a free test
    slot), in the background (`guests start`), sized as it starts: its CPUs and memory from what its
    machine has then (`size`);
  - stopped once idle (`guests watch`, every WATCH_SECONDS, run by this Mac's LaunchAgent): no
    test slot, compile slot or copy of a checkout held there for its `idle_minutes` (IDLE_MINUTES).

A guest's entry carries `"guest": {"on": PARENT, "kind": KIND, "vm": NAME}`: PARENT is "local"
(this Mac) or the ssh name of its machine (a Mac's shell, or Windows' PowerShell for WSL); KIND is
lima, orb, parallels or wsl; NAME the VM's (or WSL distribution's) own name. Optional: `cpus`,
`memory_gb` or `memory_fraction` (of the machine's memory; FRACTION, LOCAL_FRACTION here),
`idle_minutes`. A Windows host on a guest (`shares`) is that guest's.

Its record (`state/guests/NAME.json`) is what the farm reads: `state` (running, stopped, starting,
stopping), the size it started with, when it last worked. `farm.health` takes a guest that is not
running as not usable without probing it (no back-off: it is not down), and the dashboard shows
each guest on its machine.
"""
import fcntl
import glob
import json
import os
import shlex
import subprocess
import sys
import time

import farm

GB = 1 << 30
IDLE_MINUTES = 10
WATCH_SECONDS = 60
START_SECONDS = 300    # booted and answering ssh within this, else the start failed
RETRY_SECONDS = 600    # no new start this long after a failed one
FRACTION = 0.5         # of its machine's memory, at most
LOCAL_FRACTION = 0.25  # on this Mac, where the edit loop runs
RESERVE_GB = 4         # left to its machine beyond what it has available
MIN_GB = 4
HERE = os.path.dirname(os.path.realpath(__file__))
LIMA_PATH = "PATH=/opt/homebrew/bin:/usr/local/bin:$PATH; "
PRL = "PATH=/usr/local/bin:$PATH; "  # (a LaunchAgent's PATH has neither)


def log(message):
    """The guests' own log (`state/guests/log`), a line per start, stop and error: moved to `log.1`
    once over 2 MB, so the two stay under 4 MB."""
    path = farm.state("guests", "log")
    try:
        if os.path.getsize(path) > 2 << 20:
            os.replace(path, path + ".1")
    except OSError:
        pass
    with open(path, "a") as f:
        f.write(time.strftime("%Y-%m-%d %H:%M:%S ") + message + "\n")


def spec(host):
    """(the guest's own entry, its `guest` settings) for `host` (a Windows host: its machine's), or
    (None, None) when it is no guest."""
    if host is None:
        return None, None
    base = farm.shared_base(host)
    g = base.get("guest")
    return (base, g) if g else (None, None)


def guests():
    return [h for h in farm.hosts() if h.get("guest") and h.get("enabled", True)]


def path(name):
    return farm.state("guests", name + ".json")


def record(name):
    return farm.read_json(path(name), {})


def update(name, **values):
    r = record(name)
    r.update(values)
    farm.write_json(path(name), r)
    return r


def placement(g):
    """The machine a guest runs on, as the dashboard names it: this Mac, or a farm host (for WSL,
    the Windows host whose `windows_ssh` is the guest's `on`); None when it has no entry."""
    if g["on"] == "local":
        return "this Mac"
    if farm.entry(g["on"]):
        return g["on"]
    # WSL: its Windows machine's entry (the one reached at the guest's `on`).
    return next((h["name"] for h in farm.hosts() if h.get("windows_ssh") == g["on"]), None)


def on_parent(g, script, timeout=60):
    """Runs `script` on the guest's machine: sh on a Mac, PowerShell on Windows (wsl)."""
    if g["on"] == "local":
        cmd = ["/bin/sh", "-c", script]
    else:
        cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", g["on"], script]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None
    return out


# ---- each kind of guest: its state, its machine's size, and how it is sized, started and stopped

def mac_machine(g):
    """(total memory, available memory, cores) of a Mac: available is free, inactive, speculative
    and purgeable pages (what macOS hands out without swapping)."""
    out = on_parent(g, "sysctl -n hw.memsize hw.ncpu; vm_stat", 20)
    if out is None or out.returncode != 0:
        return None
    lines = out.stdout.splitlines()
    total, cores = int(lines[0]), int(lines[1])
    page = int(lines[2].split("page size of")[1].split()[0])
    pages = {}
    for line in lines[3:]:
        if ":" in line:
            k, v = line.split(":", 1)
            pages[k.strip()] = int(v.strip().rstrip(".") or 0) if v.strip().rstrip(".").isdigit() else 0
    avail = page * sum(pages.get(k, 0) for k in ("Pages free", "Pages inactive", "Pages speculative", "Pages purgeable"))
    return total, avail, cores


class Lima:
    def __init__(self, g):
        self.g, self.vm = g, shlex.quote(g["vm"])

    def state(self):
        out = on_parent(self.g, LIMA_PATH + f"limactl list --json {self.vm}", 30)
        if out is None or out.returncode != 0:
            return None
        try:
            status = json.loads(out.stdout.splitlines()[0])["status"].lower()
        except (ValueError, IndexError, KeyError):
            return None
        return status if status in ("running", "stopped") else "stopped" if status == "broken" else status

    def machine(self):
        return mac_machine(self.g)

    def size(self, cpus, gb):
        return on_parent(self.g, LIMA_PATH + f"limactl edit --tty=false {self.vm} --cpus {cpus} --memory {gb}", 60)

    def start(self):
        return on_parent(self.g, LIMA_PATH + f"limactl start --tty=false {self.vm}", START_SECONDS)

    def stop(self):
        return on_parent(self.g, LIMA_PATH + f"limactl stop {self.vm}", 180)


class Orb:
    """An OrbStack machine: OrbStack's one VM gives its memory back as it frees it, and stopping the
    machine frees what it holds. Its memory stays under OrbStack's own limit (`orb config`)."""
    ORB = "PATH=/usr/local/bin:/opt/homebrew/bin:$PATH; orb "

    def __init__(self, g):
        self.g, self.vm = g, g["vm"]

    def state(self):
        out = on_parent(self.g, self.ORB + "list -f json", 30)
        if out is None or out.returncode != 0:
            return None
        try:
            m = next((m for m in json.loads(out.stdout) if m.get("name") == self.vm), None)
        except ValueError:
            return None
        return None if m is None else m.get("state", "stopped").lower()

    def machine(self):
        m = mac_machine(self.g)
        out = on_parent(self.g, self.ORB + "config get memory_mib", 20)
        if m and out is not None and out.returncode == 0 and out.stdout.strip().isdigit():
            limit = int(out.stdout.strip()) << 20
            return min(m[0], limit), min(m[1], limit), m[2]
        return m

    def size(self, cpus, gb):
        vm = shlex.quote(self.vm)
        return on_parent(self.g, self.ORB + f"config set machine.{vm}.cpu {cpus} && " + self.ORB +
                         f"config set machine.{vm}.memory_mib {gb * 1024}", 60)

    def start(self):
        return on_parent(self.g, self.ORB + "start " + shlex.quote(self.vm), START_SECONDS)

    def stop(self):
        return on_parent(self.g, self.ORB + "stop " + shlex.quote(self.vm), 180)


class Parallels:
    """A Parallels VM: stopped by Windows' own shutdown (`prlctl stop`), killed only if that hangs."""
    def __init__(self, g):
        self.g, self.vm = g, shlex.quote(g["vm"])

    def state(self):
        out = on_parent(self.g, PRL + f"prlctl list -i {self.vm} | sed -n 's/^State: //p'", 30)
        if out is None or out.returncode != 0 or not out.stdout.strip():
            return None
        s = out.stdout.strip().lower()
        return "running" if s == "running" else "stopped" if s in ("stopped", "suspended") else s

    def machine(self):
        return mac_machine(self.g)

    def size(self, cpus, gb):
        return on_parent(self.g, PRL + f"prlctl set {self.vm} --cpus {cpus} --memsize {gb * 1024}", 60)

    def start(self):
        return on_parent(self.g, PRL + f"prlctl start {self.vm}", 120)

    def stop(self):
        out = on_parent(self.g, PRL + f"prlctl stop {self.vm}", 240)
        if out is None or out.returncode != 0:
            out = on_parent(self.g, PRL + f"prlctl stop {self.vm} --kill", 60)
        return out


class Wsl:
    """WSL on a Windows PC, through Windows' sshd (PowerShell). Started by the PC's task "Build farm
    WSL" (as the farm's account, which keeps the distribution running), stopped by ending the
    distribution and then WSL's VM (the farm account's own: a PC's user's WSL is apart). Sized in the account's `.wslconfig`,
    which WSL reads as its VM starts."""
    TASK = "Build farm WSL"

    def __init__(self, g):
        self.g, self.vm = g, g["vm"]

    def state(self):
        out = on_parent(self.g, "$env:WSL_UTF8=1; $r = @(wsl.exe --list --running --quiet) | % { $_.Trim() }; "
                        f"if ($r -contains '{self.vm}') {{ 'running' }} else {{ 'stopped' }}", 30)
        if out is None or out.returncode != 0:
            return None
        return out.stdout.strip().splitlines()[-1].strip() if out.stdout.strip() else None

    def machine(self):
        out = on_parent(self.g, "$o = Get-CimInstance Win32_OperatingSystem; $m = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory; "
                        "$c = (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors; "
                        "\"$($o.TotalVisibleMemorySize * 1024) $($m.AvailableBytes) $c\"", 30)
        if out is None or out.returncode != 0:
            return None
        try:
            total, avail, cores = (int(x) for x in out.stdout.split()[-3:])
        except ValueError:
            return None
        return total, avail, cores

    def size(self, cpus, gb):
        lines = ["[wsl2]", "networkingMode=mirrored", f"memory={gb}GB", f"processors={cpus}", "swap=8GB",
                 "vmIdleTimeout=30000", "[general]", "instanceIdleTimeout=-1", "[experimental]",
                 "autoMemoryReclaim=gradual", "sparseVhd=true"]
        value = ",".join(f"'{x}'" for x in lines)
        return on_parent(self.g, f"Set-Content -Encoding ascii -Path \"$env:USERPROFILE\\.wslconfig\" -Value @({value})", 30)

    def start(self):
        return on_parent(self.g, f"Start-ScheduledTask '{self.TASK}'", 60)

    def stop(self):
        # (then the VM: the farm account's own, whatever its `vmIdleTimeout`)
        return on_parent(self.g, f"wsl.exe --terminate {self.vm}; wsl.exe --shutdown", 120)


KINDS = {"lima": Lima, "orb": Orb, "parallels": Parallels, "wsl": Wsl}


def kind(g):
    return KINDS[g["kind"]](g)


def size(g, machine):
    """(cpus, GB) for a guest starting on a machine with `machine` (total, available, cores), or
    (None, why). Its CPUs: all the machine's (a VM's idle vCPUs take nothing; the scheduler shares
    busy ones), half on this Mac. Its memory: its share of the machine's (`memory_fraction`, or
    `memory_gb`), never more than the machine has available less RESERVE_GB."""
    total, avail, cores = machine
    cpus = g.get("cpus") or (max(2, cores // 2) if g["on"] == "local" else cores)
    want = g["memory_gb"] * GB if g.get("memory_gb") else g.get("memory_fraction", LOCAL_FRACTION if g["on"] == "local" else FRACTION) * total
    gb = int(min(want, avail - RESERVE_GB * GB) // GB)
    if gb < MIN_GB:
        return None, f"its machine has {avail / GB:.1f} GB available, under {MIN_GB + RESERVE_GB} GB"
    return cpus, gb


# ---- what the guest is doing

def lock_paths(base):
    """Every lock the farm holds while it uses `base`'s guest: its machine's test slots, the compile
    slots of it and of each host on it, and its copies of the checkouts (a Relax session holds one)."""
    names = [base["name"]] + [h["name"] for h in farm.hosts() if h.get("shares") == base["name"]]
    paths = list(farm.test_slot_paths(base["name"], base))
    for n in names:
        paths += glob.glob(os.path.join(farm.STATE, "slots", f"{n}.*"))
    paths += glob.glob(os.path.join(farm.STATE, "tests", f"{base['name']}.*.tree"))
    return paths


class Held:
    """A lock taken as `farm.Lock` takes it (the watcher's looks leave its time alone)."""
    def __init__(self, path):
        self.f = open(path, "a+")
        try:
            fcntl.flock(self.f, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.held = True
        except BlockingIOError:
            self.held = False

    def release(self):
        self.f.close()


def take_all(base):
    """Every lock of `lock_paths`, held (the guest is idle: nothing can start on it while they are),
    or None (something runs there) with the ones taken released."""
    held = []
    for p in lock_paths(base):
        lock = Held(p)
        if not lock.held:
            lock.release()
            for h in held:
                h.release()
            return None
        held.append(lock)
    return held


def last_taken(base):
    """When the farm last took one of `base`'s slots for a job (`farm.Lock.used` stamps its time), or
    sent a checkout's tree there: a job that started and ended between two of the watcher's looks."""
    times = []
    for p in lock_paths(base):
        try:
            times.append(os.stat(p).st_mtime)
        except OSError:
            pass
    return max(times, default=0)


# ---- start, stop, watch

def start(name, why="a command needs it"):
    """Starts guest `name` (sized first) and waits until it answers ssh. Run detached by
    `request_start`; one start or stop of a guest at a time (its `.lock`)."""
    host = farm.entry(name)
    base, g = spec(host)
    if g is None:
        return f"{name} is no guest"
    lock = farm.Lock(path(base["name"]) + ".lock", blocking=False)
    if not lock.held:
        lock.release()
        return "being started or stopped already"
    try:
        k = kind(g)
        now_state = k.state()
        if now_state is None:
            update(base["name"], state="unknown", why="its machine did not answer", checked=time.time())
            return "its machine did not answer"
        if now_state == "running":
            update(base["name"], state="running", since=record(base["name"]).get("since") or time.time(),
                   last_busy=time.time(), why=None, checked=time.time())
            return None
        machine = k.machine()
        if machine is None:
            update(base["name"], state="stopped", why="its machine did not answer", failed_at=time.time())
            return "its machine did not answer"
        cpus, gb = size(g, machine)
        if cpus is None:
            update(base["name"], state="stopped", why=gb, failed_at=time.time())
            return gb
        started = time.time()
        update(base["name"], state="starting", cpus=cpus, memory_gb=gb, since=started, why=why,
               machine_gb=round(machine[0] / GB, 1), checked=started)
        k.size(cpus, gb)
        out = k.start()
        started_cmd = round(time.time() - started)
        while time.time() - started < START_SECONDS:
            try:
                if subprocess.run(farm.ssh(base["name"], "true", "health"), capture_output=True, timeout=10).returncode == 0:
                    break
            except subprocess.TimeoutExpired:
                pass
            time.sleep(3)
        else:
            err = (out.stderr.strip().splitlines() or [""])[-1] if out is not None else "timed out"
            update(base["name"], state="stopped", why=f"did not come up in {START_SECONDS} s ({err})", failed_at=time.time())
            k.stop()
            return "did not come up"
        now = time.time()
        update(base["name"], state="running", since=now, last_busy=now, boot_s=round(now - started), why=None, checked=now)
        log(f"{base['name']}: guest started on {g['on']} ({cpus} CPUs, {gb} GB) in {round(now - started)} s"
            f" (its start command {started_cmd} s, then ssh)")
    finally:
        lock.release()
    # Probed now, it and the hosts on it (a Windows side): an older probe's failure from while it
    # was stopped would otherwise hold them back until its back-off ends.
    for h in [base] + [h for h in farm.hosts() if h.get("shares") == base["name"]]:
        farm.health(h, fresh=True)
    return None


def request_start(host):
    """Starts `host`'s guest in the background, unless it runs, is starting, or failed to start in
    the last RETRY_SECONDS. True when it is (now) on its way up."""
    base, g = spec(host)
    if g is None:
        return False
    r = record(base["name"])
    if r.get("state") in ("running", "starting"):
        return True
    if time.time() - r.get("failed_at", 0) < RETRY_SECONDS:
        return False
    subprocess.Popen([sys.executable, os.path.join(HERE, "guests"), "start", base["name"]], start_new_session=True,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=open(farm.state("guests", "start.err"), "a"))
    update(base["name"], state="starting", since=time.time(), why="a command needs it")
    return True


def stop(name, why):
    """Stops guest `name` if nothing of the farm's runs there: it holds every lock meanwhile."""
    base, g = spec(farm.entry(name))
    if g is None:
        return f"{name} is no guest"
    lock = farm.Lock(path(base["name"]) + ".lock", blocking=False)
    if not lock.held:
        lock.release()
        return "being started or stopped already"
    held = take_all(base)
    if held is None:
        lock.release()
        return "in use"
    try:
        update(base["name"], state="stopping", why=why)
        kind(g).stop()
        # Its ssh connections end with it: one kept open to a VM that is gone held a later start's
        # first connections back.
        for h in [base] + [h for h in farm.hosts() if h.get("shares") == base["name"]]:
            farm.close_masters(h["name"])
        update(base["name"], state="stopped", since=time.time(), why=why, checked=time.time())
        log(f"{base['name']}: guest stopped ({why})")
    finally:
        for h in held:
            h.release()
        lock.release()
    return None


def watch_once():
    now = time.time()
    for base in guests():
        g, name = base["guest"], base["name"]
        lock = farm.Lock(path(name) + ".lock", blocking=False)
        if not lock.held:  # a start or stop runs
            lock.release()
            continue
        lock.release()
        actual = kind(g).state()
        r = record(name)
        if actual is None:
            update(name, checked=now, reachable=False)
            continue
        if actual != "running":
            if r.get("state") != "stopped" or r.get("reachable") is False:
                update(name, state="stopped", since=r.get("since") if r.get("state") == "stopped" else now,
                       checked=now, reachable=True)
            else:
                update(name, checked=now)
            continue
        if r.get("state") != "running":
            r = update(name, state="running", since=now, last_busy=now, why=None)
        held = take_all(base)
        busy = held is None
        for h in held or []:
            h.release()
        last = now if busy else max(r.get("last_busy") or 0, r.get("since") or 0, last_taken(base))
        idle = now - last
        limit = 60 * g.get("idle_minutes", IDLE_MINUTES)
        update(name, last_busy=last, busy=busy, checked=now, reachable=True, stop_at=None if busy else last + limit)
        if not busy and idle >= limit:
            stop(name, f"idle {round(idle / 60)} min")


def watch():
    while True:
        try:
            watch_once()
        except Exception as e:  # noqa: BLE001 (the watcher keeps going)
            log(f"watch: {type(e).__name__}: {e}")
        time.sleep(WATCH_SECONDS)

"""Bounded process-tree CPU and residency sampling. No target instrumentation required."""
import json
import subprocess
import time


def cpu_seconds(text):
    days, _, clock = text.rpartition("-")
    parts = [float(part) for part in clock.split(":")]
    return (int(days) * 86400 if days else 0) + sum(part * 60 ** index for index, part in enumerate(reversed(parts)))


def parse_ps(text):
    rows = {}
    for line in text.splitlines():
        fields = line.split(None, 5)
        if len(fields) != 6:
            continue
        pid, parent, rss, virtual, cpu, name = fields
        rows[int(pid)] = {"pid": int(pid), "parent": int(parent), "rssBytes": int(rss) * 1024,
                          "virtualBytes": int(virtual) * 1024, "cpuSeconds": cpu_seconds(cpu), "command": name}
    return rows


class ProcessMonitor:
    def __init__(self, pid, path):
        self.pid, self.path = pid, path
        self.known = {pid}
        self.cpu = {}
        self.rows = []
        self.errors = []

    def sample(self, remaining):
        if remaining < 3:
            return
        try:
            output = subprocess.run(["ps", "-axo", "pid=,ppid=,rss=,vsz=,time=,comm="],
                                    capture_output=True, text=True, timeout=min(2, remaining - 1), check=True)
            processes = parse_ps(output.stdout)
            # Include workers whose immediate parent was discovered in this same sample.
            changed = True
            while changed:
                children = {pid for pid, row in processes.items() if row["parent"] in self.known}
                changed = not children.issubset(self.known)
                self.known.update(children)
            live = [row for pid, row in processes.items() if pid in self.known]
            for row in live:
                self.cpu[row["pid"]] = max(self.cpu.get(row["pid"], 0), row["cpuSeconds"])
            row = {"unixUs": int(time.time() * 1e6), "treeCpuSecondsLowerBound": sum(self.cpu.values()),
                   "treeRssBytes": sum(row["rssBytes"] for row in live),
                   "rootRssBytes": processes.get(self.pid, {}).get("rssBytes", 0), "processes": live}
            self.rows.append(row)
            with self.path.open("a") as stream:
                stream.write(json.dumps(row) + "\n")
        except (subprocess.SubprocessError, OSError, ValueError) as error:
            self.errors.append(str(error))

    def report(self):
        return {"samples": len(self.rows), "processesSeen": len(self.known),
                "peakTreeRssBytes": max((row["treeRssBytes"] for row in self.rows), default=0),
                "peakRootRssBytes": max((row["rootRssBytes"] for row in self.rows), default=0),
                "treeCpuSecondsLowerBound": sum(self.cpu.values()), "errors": self.errors,
                "note": "Sampled CPU is a lower bound: short-lived workers and final intervals can be missed. Summed RSS counts shared pages in each process. Use driver phase CPU and resource high-water RSS for exact single-process totals."}


def memory_snapshot(pid, tool, path, remaining):
    command = ["/usr/bin/vmmap", "-summary", str(pid)] if tool == "vmmap" else ["/usr/bin/heap", "-s", "--noContent", str(pid)]
    started = time.time()
    try:
        with path.open("w") as stream:
            result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT,
                                    timeout=min(8, remaining - 3), check=False)
        return {"tool": tool, "file": path.name, "startUnixUs": int(started * 1e6),
                "wallSeconds": time.time() - started, "exitCode": result.returncode}
    except (subprocess.SubprocessError, OSError) as error:
        return {"tool": tool, "file": path.name, "startUnixUs": int(started * 1e6),
                "wallSeconds": time.time() - started, "error": str(error)}

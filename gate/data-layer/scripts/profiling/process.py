"""Bounded process-tree CPU and residency sampling. No target instrumentation required."""
import json
import re
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

    def profile_pid(self):
        """Select the largest live workload process, including children of wrappers.

        A sample covers one PID. Preserve the choice in each capture; multi-process
        workloads still require separate captures of other significant workers.
        """
        live=self.rows[-1]['processes'] if self.rows else []
        return max(live,key=lambda row:(row['rssBytes'],row['cpuSeconds']))['pid'] if live else self.pid


def vmmap_summary(source):
    """Preserve separate rounded region and allocator views; neither is an owner census."""
    def size(value):
        match = re.fullmatch(r"(\d+(?:\.\d+)?)([KMGT]?)(?:B)?", value)
        if not match:
            raise ValueError(value)
        number, unit = match.groups()
        return int(float(number) * 1024 ** (" KMGT".index(unit) if unit else 0))

    result = {"roundedValues": True, "regions": {}, "allocatorZones": []}
    zones = False
    for line in source.splitlines():
        match = re.match(r"Physical footprint( \(peak\))?:\s+(\S+)", line)
        if match:
            key = "peakPhysicalFootprintBytesApprox" if match[1] else "physicalFootprintBytesApprox"
            result[key] = size(match[2])
        parts = re.split(r"\s{2,}", line.strip())
        if line.lstrip().startswith("MALLOC ZONE"):
            zones = True
        try:
            if not zones and parts[0].startswith("MALLOC") and len(parts) >= 5:
                result["regions"][parts[0]] = dict(zip(
                    ("virtualBytesApprox", "residentBytesApprox", "dirtyBytesApprox", "swappedBytesApprox"),
                    map(size, parts[1:5])))
            elif zones and len(parts) >= 7:
                result["allocatorZones"].append({"name": parts[0], "virtualBytesApprox": size(parts[1]),
                    "residentBytesApprox": size(parts[2]), "dirtyBytesApprox": size(parts[3]),
                    "swappedBytesApprox": size(parts[4]), "allocationCount": int(parts[5]),
                    "allocatedBytesApprox": size(parts[6])})
        except ValueError:
            continue
    result["recognized"] = "physicalFootprintBytesApprox" in result or bool(result["regions"])
    return result


def memory_snapshot(pid, tool, path, remaining):
    command = ["/usr/bin/vmmap", "-summary", str(pid)] if tool == "vmmap" else ["/usr/bin/heap", "-s", "--noContent", str(pid)]
    started = time.time()
    try:
        with path.open("w") as stream:
            result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT,
                                    timeout=min(8, remaining - 3), check=False)
        report = {"tool": tool, "file": path.name, "startUnixUs": int(started * 1e6),
                  "wallSeconds": time.time() - started, "exitCode": result.returncode}
        if tool == "vmmap" and result.returncode == 0:
            report["summary"] = vmmap_summary(path.read_text(errors="replace"))
        return report
    except (subprocess.SubprocessError, OSError) as error:
        return {"tool": tool, "file": path.name, "startUnixUs": int(started * 1e6),
                "wallSeconds": time.time() - started, "error": str(error)}

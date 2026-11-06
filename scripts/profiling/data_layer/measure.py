#!/usr/bin/env python3
"""Measure an authorized complete run, including waited-for descendant CPU."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from process import ProcessMonitor


class DiskSpace:
    def __init__(self, path):
        self.path = path
        self.initial = self.minimum = self.final = self.total = None
        self.samples = 0
        self.errors = []
        self.sample()

    def sample(self):
        try:
            usage = shutil.disk_usage(self.path)
            self.total, self.final = usage.total, usage.free
            if self.initial is None:
                self.initial = usage.free
            self.minimum = usage.free if self.minimum is None else min(self.minimum, usage.free)
            self.samples += 1
        except OSError as error:
            if len(self.errors) < 16:
                self.errors.append(str(error))

    def report(self):
        return dict(path=str(self.path), totalBytes=self.total, initialFreeBytes=self.initial,
                    minimumFreeBytes=self.minimum, finalFreeBytes=self.final, samples=self.samples, errors=self.errors)


def measure(command, output):
    started = time.monotonic()
    disks = dict(workspace=DiskSpace(Path.cwd()),
                 temporary=DiskSpace(Path(os.environ.get('SQLITE_TMPDIR') or tempfile.gettempdir())))
    with output.with_suffix('.stdout.log').open('w') as stdout, output.with_suffix('.stderr.log').open('w') as stderr:
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        monitor = ProcessMonitor(process.pid, output.with_suffix('.processes.jsonl'))
        while True:
            for disk in disks.values(): disk.sample()
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid:
                process.returncode = os.waitstatus_to_exitcode(status)
                break
            monitor.sample(10)
            time.sleep(1)
        report = dict(format='rerail.complete-process-measurement/v1', command=command, exitCode=process.returncode,
                      wallSeconds=time.monotonic()-started, userCpuSeconds=usage.ru_utime, systemCpuSeconds=usage.ru_stime,
                      cpuSeconds=usage.ru_utime+usage.ru_stime,
                      filesystems={name: disk.report() for name, disk in disks.items()},
                      maxSingleProcessRssBytes=usage.ru_maxrss*(1 if sys.platform=='darwin' else 1024), processTree=monitor.report(),
                      note='wait4 CPU includes the command and descendants it waited for. maxRSS is the largest process high-water mark, not concurrent family RSS. Concurrent family RSS is sampled and counts shared pages repeatedly. Native driver records exact root peak RSS through close. No execution timeout: use only for authorized complete repository gates.')
        output.write_text(json.dumps(report, indent=2)+'\n')
        print(json.dumps(report, indent=2))
        return process.returncode


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args=parser.parse_args()
    command=args.command[1:] if args.command[:1]==['--'] else args.command
    if not command: parser.error('A command is required after --')
    raise SystemExit(measure(command,args.output))

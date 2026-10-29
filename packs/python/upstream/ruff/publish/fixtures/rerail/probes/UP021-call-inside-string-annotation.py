import subprocess

x: "subprocess.run('ls', universal_newlines=True)"
y: "subprocess.run('ls', universal_newlines=True, text=True)"

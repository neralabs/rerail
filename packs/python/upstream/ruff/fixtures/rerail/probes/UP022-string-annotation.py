import subprocess
x: "subprocess.run(['a'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)"
y: "list[subprocess.run(['a'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, capture_output=True)]"
w = "subprocess.run(['a'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)"

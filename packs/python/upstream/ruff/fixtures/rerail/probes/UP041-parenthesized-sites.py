import socket
from socket import timeout

try:
    pass
except (socket.timeout):
    pass

try:
    pass
except ((socket.timeout), KeyError) as e:
    pass

try:
    pass
except ((  # comment
    timeout,
)):
    pass

try:
    pass
except (KeyError, (ValueError, socket.timeout)):
    pass

raise (socket
       .timeout)
raise KeyError from socket.timeout
raise socket.timeout()
x = (socket.timeout)("message")
y = [socket.timeout]

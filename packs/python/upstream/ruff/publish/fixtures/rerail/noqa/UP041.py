import socket

try:
    pass
except socket.timeout:
    pass

import socket

try:
    pass
except socket.timeout:  # noqa
    pass

import socket

try:
    pass
except socket.timeout:  # noqa: UP041
    pass

import socket

try:
    pass
except socket.timeout:  # NOQA: UP041
    pass

import socket

try:
    pass
except socket.timeout:  # noqa: E501
    pass

import ipaddress
import socket
from urllib.parse import urlparse


def validate_url(url):
    host = urlparse(url).hostname or ""
    return all(ipaddress.ip_address(info[4][0]).is_global for info in socket.getaddrinfo(host, None))


def is_allowed_host(host):
    return host in {"hooks.example.com"}

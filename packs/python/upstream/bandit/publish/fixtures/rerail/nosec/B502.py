import ssl
ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)

import ssl
ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)  # nosec

import ssl
ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)  # nosec B502

import ssl
ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)  # nosec: B502

import ssl
ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)  # nosec ssl_with_bad_version

import ssl
ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)  # nosec B101

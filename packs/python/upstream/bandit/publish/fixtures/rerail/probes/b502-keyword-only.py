import ssl
from OpenSSL import SSL
SSL.Context(SSL.SSLv23_METHOD)
ssl.SSLContext(ssl.PROTOCOL_TLSv1)
SSL.Context(method=SSL.SSLv23_METHOD)
ssl.wrap_socket(sock, ssl_version=ssl.PROTOCOL_TLSv1)

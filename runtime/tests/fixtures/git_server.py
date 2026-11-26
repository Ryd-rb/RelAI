"""Private Smart HTTPS test server backed by the installed Git reference implementation."""
import http.server
import os
import ssl
import subprocess
import sys
import urllib.parse

root, cert, key = sys.argv[1:]

class Backend(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def do_GET(self):
        self.exchange()

    def do_POST(self):
        self.exchange()

    def exchange(self):
        target = urllib.parse.urlsplit(self.path)
        size = int(self.headers.get("Content-Length", 0))
        if not 0 <= size <= 34 * 1024 * 1024:
            self.send_error(413)
            return
        env = dict(os.environ, GIT_PROJECT_ROOT=root, GIT_HTTP_EXPORT_ALL="1",
                   REQUEST_METHOD=self.command, PATH_INFO=target.path, QUERY_STRING=target.query,
                   CONTENT_TYPE=self.headers.get("Content-Type", ""), CONTENT_LENGTH=str(size),
                   REMOTE_USER="fixture", REMOTE_ADDR="127.0.0.1",
                   GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        reply = subprocess.run(["git", "http-backend"], input=self.rfile.read(size),
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, timeout=15, check=True)
        headers, payload = reply.stdout.split(b"\r\n\r\n", 1)
        fields = [line.decode("ascii").split(": ",1) for line in headers.split(b"\r\n")]
        self.send_response(next((int(v.split()[0]) for k,v in fields if k.lower()=="status"),200))
        for name, value in fields:
            if name.lower() != "status":
                self.send_header(name,value)
        self.send_header("Content-Length",str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(("127.0.0.1",0),Backend)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert,key)
server.socket=context.wrap_socket(server.socket,server_side=True)
print(server.server_port, flush=True)
server.serve_forever()

"""Loopback HTTPS fixture for the real Rust transport; no external requests."""
import base64
import http.server
import json
import pathlib
import ssl
import sys
import threading
import time

root = pathlib.Path(sys.argv[1])
lock = threading.Lock()
route_counts = {}


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.respond()

    def do_POST(self):
        self.respond()

    def respond(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        with lock:
            with (root / "requests.jsonl").open("a") as output:
                output.write(
                    json.dumps(
                        {
                            "path": self.path,
                            "method": self.command,
                            "headers": dict(self.headers),
                            "body": base64.b64encode(body).decode(),
                        }
                    )
                    + "\n"
                )
        state = json.loads((root / "response.json").read_text())
        if "routes" in state:
            with lock:
                key = self.command + " " + self.path
                route = state["routes"].get(
                    key, state["routes"].get(self.path, {"status": 404})
                )
                index = route_counts.get(key, 0)
                route_counts[key] = index + 1
                sequence = route.get("responses", [route])
                state = sequence[min(index, len(sequence) - 1)]
        time.sleep(state.get("delay", 0))
        payload = base64.b64decode(state.get("body", ""))
        try:
            self.send_response(state.get("status", 200))
            for key, value in state.get("headers", {}).items():
                self.send_header(key, value)
            self.send_header("Content-Length", len(payload))
            self.end_headers()
            self.wfile.write(payload[: state.get("truncate", len(payload))])
        except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
            pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root / "cert.pem", root / "key.pem")
server.socket = context.wrap_socket(server.socket, server_side=True)
print(server.server_port, flush=True)
server.serve_forever()

"""Run a command against an isolated, in-memory Blob/DFS HTTP contract fixture.

This exercises the adapter through real SDK requests. Use a real HNS account to
test Azure's service implementation, RBAC, private networking, and scale.
"""

import base64
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass, field
from email.utils import formatdate
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import RLock, Thread
from urllib.parse import parse_qs, unquote, urlsplit
from uuid import uuid4
from xml.etree import ElementTree


@dataclass
class Node:
    directory: bool
    data: bytes = b""
    meta: dict = field(default_factory=dict)
    etag: str = field(default_factory=lambda: f'"{uuid4()}"')
    modified: str = field(default_factory=lambda: formatdate(usegmt=True))
    transaction: str = ""


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def respond(self, status, headers=None, data=b""):
        self.send_response(status)
        headers = headers or {}
        for key, value in headers.items():
            self.send_header(key, value)
        if "Content-Length" not in headers:
            self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(data)

    def error(self, status, code):
        self.respond(
            status,
            {"x-ms-error-code": code, "Content-Type": "application/json"},
            json.dumps({"error": {"code": code, "message": code}}).encode(),
        )

    def handle_request(self):
        path = unquote(urlsplit(self.path).path)
        probe = None
        with self.server.guard:
            if self.command == "HEAD":
                for prefix, metrics in self.server.probes.items():
                    if path.startswith(prefix + "/"):
                        probe = metrics
                        probe["heads"] += 1
                        probe["active"] += 1
                        probe["peak"] = max(probe["peak"], probe["active"])
                        break
        try:
            if probe is not None:
                # Outside the state lock: make overlapping metadata requests observable.
                time.sleep(0.02)
            with self.server.guard:
                if probe is not None and path == probe.get("missing"):
                    return self.error(404, "PathNotFound")
                if probe is not None and path == probe.get("denied"):
                    return self.error(403, "AuthorizationFailure")
                self.dispatch()
        except (BrokenPipeError, ConnectionResetError):
            # A failed metadata request cancels its sibling requests on the client.
            pass
        finally:
            if probe is not None:
                with self.server.guard:
                    probe["active"] -= 1

    do_GET = do_HEAD = do_PUT = do_DELETE = do_PATCH = handle_request

    def dispatch(self):
        nodes = self.server.nodes
        url = urlsplit(self.path)
        query = {k: v[0] for k, v in parse_qs(url.query).items()}
        path = unquote(url.path)
        if path == "/__fixture/metrics":
            prefix = query["prefix"]
            if self.command == "DELETE":
                self.server.probes[prefix] = {
                    "heads": 0,
                    "active": 0,
                    "peak": 0,
                    "missing": query.get("missing"),
                    "denied": query.get("denied"),
                }
            return self.respond(
                200,
                {"Content-Type": "application/json"},
                json.dumps(self.server.probes[prefix]).encode(),
            )
        if query.get("comp") == "properties" and query.get("restype") == "account":
            return self.respond(200, {"x-ms-is-hns-enabled": "true"})
        if not (path == "/ov-test" or path.startswith("/ov-test/")):
            return self.error(404, "FilesystemNotFound")
        node = nodes.get(path)
        if query.get("resource") == "filesystem":
            root = "/ov-test/" + query["directory"]
            if root not in nodes:
                return self.error(404, "PathNotFound")
            names = sorted(
                k
                for k in nodes
                if k.startswith(root + "/")
                and (query["recursive"] == "true" or "/" not in k[len(root) + 1 :])
            )
            offset = int(query.get("continuation", 0))
            size = min(32, int(query["maxResults"]))
            listing = [
                {
                    "name": k[len("/ov-test/") :],
                    "lastModified": nodes[k].modified,
                    "contentLength": len(nodes[k].data),
                    **({"isDirectory": "true"} if nodes[k].directory else {}),
                }
                for k in names[offset : offset + size]
            ]
            headers = {"Content-Type": "application/json"}
            if offset + size < len(names):
                headers["x-ms-continuation"] = str(offset + size)
            return self.respond(200, headers, json.dumps({"paths": listing}).encode())
        if self.headers.get("If-None-Match") == "*" and node:
            return self.error(412, "ConditionNotMet")
        expected = self.headers.get("If-Match")
        if expected and (not node or expected != node.etag):
            return self.error(412, "ConditionNotMet")
        if self.command == "PUT":
            if query.get("comp") == "block":
                block = self.rfile.read(int(self.headers["Content-Length"]))
                self.server.blocks[path, query["blockid"]] = block
                return self.respond(201)
            if query.get("comp") == "blocklist":
                body = ElementTree.fromstring(self.rfile.read(int(self.headers["Content-Length"])))
                data = b"".join(self.server.blocks.pop((path, b.text)) for b in body)
                meta = {
                    k.lower()[10:]: v
                    for k, v in self.headers.items()
                    if k.lower().startswith("x-ms-meta-")
                }
                nodes[path] = Node(False, data, meta)
                return self.respond(
                    201, {"etag": nodes[path].etag, "Last-Modified": nodes[path].modified}
                )
            source = self.headers.get("x-ms-rename-source")
            if source:
                source = unquote(source)
                continuation = query.get("continuation")
                if continuation:
                    pending = self.server.renames.pop(continuation, None)
                    if pending != (source, path, self.headers["x-ms-client-transaction-id"]):
                        return self.error(400, "InvalidContinuation")
                    return self.respond(201, {"etag": node.etag})
                original = nodes.get(source)
                if not original:
                    return self.error(404, "SourcePathNotFound")
                if self.headers.get("x-ms-source-if-match") != original.etag:
                    return self.error(412, "SourceConditionNotMet")
                if not expected and self.headers.get("If-None-Match") != "*":
                    return self.error(400, "MissingDestinationCondition")
                if query.get("mode") != "posix":
                    return self.error(400, "InvalidRenameMode")
                for old in [k for k in nodes if k == source or k.startswith(source + "/")]:
                    nodes[path + old[len(source) :]] = nodes.pop(old)
                original.transaction = self.headers["x-ms-client-transaction-id"]
                if path.endswith("/moved") and self.server.fault == "lost-rename-response":
                    self.server.fault = ""
                    return self.error(500, "InternalError")
                if original.directory:
                    token = str(uuid4())
                    self.server.renames[token] = (source, path, original.transaction)
                    return self.respond(201, {"etag": original.etag, "x-ms-continuation": token})
                return self.respond(201, {"etag": original.etag})
            if query.get("comp") == "metadata":
                if not node:
                    return self.error(404, "BlobNotFound")
                node.meta = {
                    k.lower()[10:]: v
                    for k, v in self.headers.items()
                    if k.lower().startswith("x-ms-meta-")
                }
                node.etag = f'"{uuid4()}"'
                return self.respond(200, {"etag": node.etag})
            parent = nodes.get(path.rsplit("/", 1)[0])
            if not parent or not parent.directory:
                return self.error(409, "ParentNotFound")
            if query.get("resource") == "directory":
                props = self.headers.get("x-ms-properties", "")
                meta = {
                    k: base64.b64decode(v).decode()
                    for k, v in (p.split("=", 1) for p in props.split(",") if p)
                }
                node = Node(True, meta=meta)
            elif self.headers.get("x-ms-blob-type") == "BlockBlob":
                if node and node.directory:
                    return self.error(409, "PathConflict")
                data = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                meta = {
                    k.lower()[10:]: v
                    for k, v in self.headers.items()
                    if k.lower().startswith("x-ms-meta-")
                }
                node = Node(False, data, meta)
            else:
                return self.error(400, "UnexpectedWrite")
            nodes[path] = node
            return self.respond(201, {"etag": node.etag, "Last-Modified": node.modified})
        if not node:
            return self.error(404, "PathNotFound")
        if self.command == "DELETE":
            children = [k for k in nodes if k.startswith(path + "/")]
            if children and query.get("recursive") != "true":
                return self.error(409, "DirectoryNotEmpty")
            if children and not query.get("continuation"):
                if query.get("paginated") != "true":
                    return self.error(400, "MissingPagination")
                token = str(uuid4())
                self.server.deletes[token] = path
                return self.respond(200, {"x-ms-continuation": token})
            if (
                query.get("continuation")
                and self.server.deletes.pop(query["continuation"], None) != path
            ):
                return self.error(400, "InvalidContinuation")
            for k in [path, *children]:
                del nodes[k]
            return self.respond(200 if node.directory else 202)
        headers = {
            "etag": node.etag,
            "Last-Modified": node.modified,
            "x-ms-blob-type": "BlockBlob",
            "x-ms-resource-type": "directory" if node.directory else "file",
            "x-ms-properties": ",".join(
                k + "=" + base64.b64encode(v.encode()).decode() for k, v in node.meta.items()
            ),
            "x-ms-client-transaction-id": node.transaction,
        }
        if self.command == "HEAD":
            return self.respond(200, {**headers, "Content-Length": str(len(node.data))})
        if self.command != "GET":
            return self.error(400, "UnsupportedRequest")
        data = node.data
        byte_range = self.headers.get("x-ms-range") or self.headers.get("range")
        if data and byte_range:
            lo, hi = byte_range.removeprefix("bytes=").split("-")
            start = int(lo)
            end = min(int(hi) if hi else len(data) - 1, len(data) - 1)
            headers["Content-Range"] = f"bytes {start}-{end}/{len(data)}"
            return self.respond(206, headers, data[start : end + 1])
        self.respond(200, headers, data)


def main():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.blocks = {}
    server.probes = {}
    server.renames = {}
    server.deletes = {}
    server.fault = os.environ.get("OV_HNS_FIXTURE_FAULT", "")
    server.guard = RLock()
    server.nodes = {"/ov-test": Node(True)}
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        return subprocess.run(
            sys.argv[1:],
            env={
                **os.environ,
                "OV_AZURE_TEST_NAMESPACE": "hns",
                "OV_AZURE_TEST_URL": f"http://127.0.0.1:{server.server_port}",
                "OV_AZURE_TEST_CONTAINER": "ov-test",
                "OV_AZURE_TEST_SAS": "fixture=1",
                "OV_HNS_FIXTURE": "1",
            },
        ).returncode
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    raise SystemExit(main())

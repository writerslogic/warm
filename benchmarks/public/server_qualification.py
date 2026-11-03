# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Local HTTP integrity soak with durable client receipts and an explicit freeze."""

import argparse
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import platform
import secrets
import signal
import socket
import subprocess
import threading
import time
import traceback


GROUPS = 8
BATCH = 32
DIM = 16
TENANTS = 4


def digest(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    with open(path, "x", encoding="utf8") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def allocated_bytes(root):
    total = 0
    for path in root.rglob("*"):
        try:
            if path.is_file():
                total += path.stat().st_blocks * 512
        except FileNotFoundError:
            continue
    return total


def rss_bytes(pid):
    status = Path(f"/proc/{pid}/status").read_text()
    return int(next(line.split()[1] for line in status.splitlines() if line.startswith("VmRSS:"))) * 1024


def document(tenant, group, revision, index):
    return {
        "id": f"doc-{group * BATCH + index:04d}" if group >= 0 else f"churn-{revision}",
        "text": f"tenant {tenant} group {group} revision {revision} member {index}",
        "vector": [float(1 + (index + dimension) % 7) for dimension in range(DIM)],
        "metadata": {"tenant": tenant, "group": group, "revision": revision, "member": index, "bucket": group % 2},
    }


def validate_matches(matches, tenant, minimum, maximum, deleted, bucket=None):
    ids = set()
    groups = {}
    for row in matches:
        identifier = row["id"]
        if identifier in ids or identifier in deleted:
            raise AssertionError("duplicate or acknowledged-deleted result")
        ids.add(identifier)
        metadata = row["metadata"]
        if metadata["tenant"] != tenant:
            raise AssertionError("cross-tenant result")
        group, revision, member = (metadata[key] for key in ("group", "revision", "member"))
        if not all(type(value) is int for value in (group, revision, member)):
            raise AssertionError("invalid result metadata types")
        if not (-1 <= group < GROUPS and revision >= 0 and 0 <= member < BATCH):
            raise AssertionError("invalid result metadata range")
        expected = document(tenant, group, revision, member)
        if identifier != expected["id"] or row["text"] != expected["text"] or metadata != expected["metadata"]:
            raise AssertionError("incorrect result membership or payload")
        if not math.isfinite(row["score"]) or not -1.000001 <= row["score"] <= 1.000001:
            raise AssertionError("invalid score")
        if bucket is not None and metadata["bucket"] != bucket:
            raise AssertionError("filter violation")
        if group >= 0:
            groups.setdefault(group, []).append((revision, member))
    expected_groups = {group for group in range(GROUPS) if bucket is None or group % 2 == bucket}
    if set(groups) != expected_groups:
        raise AssertionError("lost acknowledged batch")
    for group, rows in groups.items():
        revisions = {revision for revision, _ in rows}
        if len(rows) != BATCH or {member for _, member in rows} != set(range(BATCH)) or len(revisions) != 1:
            raise AssertionError("partial or mixed batch")
        revision = next(iter(revisions))
        if not minimum[group] <= revision <= maximum[group]:
            raise AssertionError("lost acknowledged replacement or unissued revision")


class Client:
    def __init__(self, port, tenant, token, log):
        self.port, self.tenant, self.token, self.log = port, tenant, token, log

    def request(self, method, path, payload=None, key=None, tenant=None, expected=(200,)):
        body = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
        headers = {"Authorization": f"Bearer {self.token}", "X-Tenant-ID": tenant or self.tenant}
        if body is not None:
            headers["Content-Type"] = "application/json"
        if key:
            headers["Idempotency-Key"] = key
        start = time.monotonic()
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        try:
            connection.request(method, path, body, headers)
            response = connection.getresponse()
            raw = response.read(4 * 1024 * 1024 + 1)
            if len(raw) > 4 * 1024 * 1024:
                raise AssertionError("oversized response")
            status, replayed = response.status, response.getheader("idempotent-replayed")
            value = json.loads(raw)
        finally:
            connection.close()
        elapsed = time.monotonic() - start
        self.log.write(json.dumps({"type": "request", "time": time.time(), "method": method, "path": path,
                                   "status": status, "seconds": elapsed, "retry": key is not None}) + "\n")
        if status not in expected:
            raise AssertionError(f"unexpected HTTP status {status} for {method} {path}: {value.get('code')}")
        if elapsed > 5:
            raise AssertionError(f"request exceeded 5-second deadline: {method} {path}")
        return value, replayed

    def query(self, bucket=None):
        payload = {"query_vector": [1.0] * DIM, "top_k": 512, "index": "exact"}
        if bucket is not None:
            payload["filter"] = {"bucket": bucket}
        return self.request("POST", "/api/v1/query", payload)[0]["matches"]


class Tenant:
    def __init__(self, name, token, port, output):
        self.name, self.token, self.port, self.output = name, token, port, output
        self.lock = threading.Lock()
        self.acknowledged = [0] * GROUPS
        self.issued = [0] * GROUPS
        self.deleted = frozenset()
        self.previous = {}

    def bounds(self):
        with self.lock:
            return self.acknowledged.copy(), self.issued.copy(), self.deleted

    def acknowledge(self, ledger, operation):
        ledger.write(json.dumps({"type": "ack", "time": time.time(), **operation}) + "\n")
        ledger.flush()
        os.fsync(ledger.fileno())
        with self.lock:
            if operation["operation"] == "batch":
                self.acknowledged[operation["group"]] = operation["revision"]
            elif operation["operation"] == "delete":
                self.deleted = self.deleted | {operation["id"]}

    def batch(self, client, ledger, group, revision):
        payload = [document(self.name, group, revision, member) for member in range(BATCH)]
        key = f"batch-{group}-{revision}"
        with self.lock:
            self.issued[group] = revision
        value, _ = client.request("POST", "/api/v1/documents/batch", payload, key)
        if value != {"added": BATCH}:
            raise AssertionError("incorrect batch acknowledgment")
        self.acknowledge(ledger, {"operation": "batch", "group": group, "revision": revision, "key": key})
        previous = self.previous.get(group)
        if previous:
            old_payload, old_key, recorded_at = previous
            if time.monotonic() - recorded_at < 240:
                value, replayed = client.request("POST", "/api/v1/documents/batch", old_payload, old_key, expected=(200, 409))
                if value != {"added": BATCH} or replayed != "true":
                    if value.get("code") != "idempotency_reconciliation_required":
                        raise AssertionError("retry neither replayed nor protected for reconciliation")
        self.previous[group] = payload, key, time.monotonic()

    def initialize(self):
        with open(self.output / f"{self.name}-setup.ndjson", "x", buffering=1) as log, open(
            self.output / f"{self.name}-acks.ndjson", "x", buffering=1
        ) as ledger:
            client = Client(self.port, self.name, self.token, log)
            for group in range(GROUPS):
                self.batch(client, ledger, group, 0)
            validate_matches(client.query(), self.name, [0] * GROUPS, [0] * GROUPS, set())

    def writer(self, stop):
        with open(self.output / f"{self.name}-writer.ndjson", "x", buffering=1) as log, open(
            self.output / f"{self.name}-acks.ndjson", "a", buffering=1
        ) as ledger:
            client = Client(self.port, self.name, self.token, log)
            revision = 0
            while not stop.is_set():
                started = time.monotonic()
                revision += 1
                group = revision % GROUPS
                self.batch(client, ledger, group, revision)
                if revision % 5 == 0:
                    payload = document(self.name, -1, revision, 0)
                    client.request("POST", "/api/v1/documents", payload, f"churn-{revision}")
                    self.acknowledge(ledger, {"operation": "insert", "id": payload["id"], "revision": revision})
                    value, _ = client.request("DELETE", f"/api/v1/documents/{payload['id']}")
                    if value != {"deleted": True}:
                        raise AssertionError("churn deletion was not acknowledged")
                    self.acknowledge(ledger, {"operation": "delete", "id": payload["id"]})
                if revision % 20 == 0:
                    client.request("GET", "/api/v1/stats", tenant="unauthorized-tenant", expected=(403,))
                    client.request("POST", "/api/v1/index/maintain", {})
                minimum, maximum, deleted = self.bounds()
                validate_matches(client.query(), self.name, minimum, maximum, deleted)
                stop.wait(max(0, 1.0 - (time.monotonic() - started)))

    def reader(self, stop):
        with open(self.output / f"{self.name}-reader.ndjson", "x", buffering=1) as log:
            client = Client(self.port, self.name, self.token, log)
            iteration = 0
            while not stop.is_set():
                minimum, _, deleted = self.bounds()
                bucket = None if iteration % 3 == 0 else iteration % 2
                matches = client.query(bucket)
                _, maximum, _ = self.bounds()
                validate_matches(matches, self.name, minimum, maximum, deleted, bucket)
                iteration += 1
                stop.wait(0.5)

    def verify_ledger(self, log):
        revisions, deleted, present = [-1] * GROUPS, set(), set()
        with open(self.output / f"{self.name}-acks.ndjson") as ledger:
            for line in ledger:
                row = json.loads(line)
                if row["operation"] == "batch":
                    revisions[row["group"]] = row["revision"]
                elif row["operation"] == "delete":
                    deleted.add(row["id"])
                    present.discard(row["id"])
                elif row["operation"] == "insert":
                    present.add(row["id"])
                else:
                    raise AssertionError("unknown ledger operation")
        client = Client(self.port, self.name, self.token, log)
        matches = client.query()
        validate_matches(matches, self.name, revisions, revisions, deleted)
        if {row["id"] for row in matches if row["metadata"]["group"] < 0} != present:
            raise AssertionError("churn membership differs from durable acknowledgments")
        return client, revisions, deleted


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--seconds", type=int, default=86400)
    parser.add_argument("--development", action="store_true")
    args = parser.parse_args()
    if args.seconds < 1 or (not args.development and args.seconds < 86400):
        parser.error("qualification needs at least 86400 seconds; shorter runs require --development")
    if platform.system() != "Linux":
        parser.error("this resource monitor currently requires Linux /proc")
    args.binary = args.binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.mkdir(mode=0o700)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    credentials = [(f"tenant-{i}", f"warm_soak{i}.{secrets.token_hex(32)}") for i in range(TENANTS)]
    keys = {"keys": [{"id": f"soak{i}", "hash": "sha256:" + hashlib.sha256(token.encode()).hexdigest(),
                       "tenants": [name], "scopes": ["admin"]} for i, (name, token) in enumerate(credentials)]}
    write_json(args.output / "keys.json", keys)
    os.chmod(args.output / "keys.json", 0o600)
    command = [str(args.binary), "--bind", f"127.0.0.1:{port}", "--data-dir", str(args.output / "data"),
               "--keys-file", str(args.output / "keys.json"), "--input-dim", str(DIM), "--dim", "1024",
               "--max-top-k", "512", "--max-tenants", "4", "--compaction-interval-seconds", "60",
               "--max-concurrent-work", "8", "--max-queue", "32",
               "--idempotency-ttl-seconds", "300", "--log-level", "warn"]
    frozen = {"schema": "warm-server-qualification/1", "development": args.development,
              "sourceRevision": args.source_revision, "binarySha256": digest(args.binary),
              "harnessSha256": digest(__file__), "seconds": args.seconds, "command": command,
              "tenants": TENANTS, "groupsPerTenant": GROUPS, "atomicBatchSize": BATCH,
              "initialDocuments": TENANTS * GROUPS * BATCH, "writerPeriodSeconds": 1,
              "readerPeriodSeconds": 0.5, "host": platform.uname()._asdict(),
              "environment": {"RAYON_NUM_THREADS": "2"},
              "rssLimitBytes": 2 * 1024**3, "allocatedStoreLimitBytes": 8 * 1024**3,
              "p95LimitSeconds": 0.25, "p99LimitSeconds": 1, "requestDeadlineSeconds": 5,
              "recoveryLimitSeconds": 60, "startedUtc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    write_json(args.output / "freeze.json", frozen)
    child = None
    stop = threading.Event()
    failures, threads = [], []
    lock = threading.Lock()
    server_log = open(args.output / "server.log", "xb", buffering=0)
    start = None
    recovery = []

    def launch():
        environment = {key: value for key, value in os.environ.items() if not key.startswith(("WARM_", "HMS_"))}
        environment.update(frozen["environment"])
        process = subprocess.Popen(command, stdout=server_log, stderr=server_log,
                                   start_new_session=True, env=environment)
        deadline = time.monotonic() + frozen["recoveryLimitSeconds"]
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f"server exited during start: {process.returncode}")
            try:
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=1)
                connection.request("GET", "/readyz")
                response = connection.getresponse()
                response.read()
                ready = response.status == 200
                connection.close()
                if ready:
                    return process
            except OSError:
                pass
            time.sleep(0.05)
        process.kill()
        process.wait()
        raise RuntimeError("server exceeded recovery deadline")

    def run_worker(function):
        try:
            function(stop)
        except Exception:
            with lock:
                failures.append(traceback.format_exc())
            stop.set()

    try:
        child = launch()
        tenants = [Tenant(name, token, port, args.output) for name, token in credentials]
        for tenant in tenants:
            tenant.initialize()
        start = time.monotonic()
        for tenant in tenants:
            for function in (tenant.writer, tenant.reader):
                thread = threading.Thread(target=run_worker, args=(function,))
                thread.start()
                threads.append(thread)
        with open(args.output / "resources.ndjson", "x", buffering=1) as samples:
            while not stop.is_set() and time.monotonic() - start < args.seconds:
                if child.poll() is not None:
                    raise RuntimeError("server exited during soak")
                rss = rss_bytes(child.pid) + rss_bytes(os.getpid())
                allocated = allocated_bytes(args.output / "data")
                samples.write(json.dumps({"elapsed": time.monotonic() - start, "rssBytes": rss,
                                          "allocatedStoreBytes": allocated, "pid": child.pid}) + "\n")
                if rss > frozen["rssLimitBytes"] or allocated > frozen["allocatedStoreLimitBytes"]:
                    raise AssertionError("declared resource limit exceeded")
                stop.wait(min(5, max(0, args.seconds - (time.monotonic() - start))))
        stop.set()
        for thread in threads:
            thread.join(timeout=15)
            if thread.is_alive():
                raise RuntimeError("worker did not terminate")
        if failures:
            raise RuntimeError("workload failure; see failures.json")
        with open(args.output / "recovery.ndjson", "x", buffering=1) as log:
            for tenant in tenants:
                tenant.verify_ledger(log)
            for mode in ("sigkill", "sigterm"):
                child.send_signal(signal.SIGKILL if mode == "sigkill" else signal.SIGTERM)
                child.wait(timeout=60)
                recovery_start = time.monotonic()
                child = launch()
                for tenant in tenants:
                    client, revisions, deleted = tenant.verify_ledger(log)
                    payload, key, _ = tenant.previous[max(tenant.previous)]
                    value, replayed = client.request("POST", "/api/v1/documents/batch", payload, key, expected=(200, 409))
                    if not (value == {"added": BATCH} and replayed == "true") and value.get("code") != "idempotency_reconciliation_required":
                        raise AssertionError("restart retry was not protected")
                    validate_matches(client.query(), tenant.name, revisions, revisions, deleted)
                elapsed = time.monotonic() - recovery_start
                if elapsed > frozen["recoveryLimitSeconds"]:
                    raise AssertionError("recovery verification exceeded deadline")
                recovery.append({"mode": mode, "seconds": elapsed, "durableLedgerMatched": True})
    except Exception:
        failures.append(traceback.format_exc())
    finally:
        stop.set()
        for thread in threads:
            thread.join(timeout=15)
        if child is not None and child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=60)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
                failures.append("server failed graceful teardown")
        server_log.close()
    latencies = []
    for path in args.output.glob("*-writer.ndjson"):
        latencies.extend(json.loads(line)["seconds"] for line in path.open())
    for path in args.output.glob("*-reader.ndjson"):
        latencies.extend(json.loads(line)["seconds"] for line in path.open())
    latencies.sort()
    percentiles = {name: latencies[max(0, math.ceil(len(latencies) * fraction) - 1)] if latencies else None
                   for name, fraction in (("p50", 0.5), ("p95", 0.95), ("p99", 0.99))}
    if not latencies or percentiles["p95"] > frozen["p95LimitSeconds"] or percentiles["p99"] > frozen["p99LimitSeconds"]:
        failures.append("declared latency limit failed")
    write_json(args.output / "failures.json", failures)
    write_json(args.output / "finished.json", {"complete": not failures, "qualified24Hours": not failures and not args.development,
               "elapsedSeconds": None if start is None else time.monotonic() - start,
               "requestCount": len(latencies), "latencySeconds": percentiles, "recovery": recovery,
               "freezeSha256": digest(args.output / "freeze.json"), "failureCount": len(failures)})
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()

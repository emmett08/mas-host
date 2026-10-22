"""Real socket framing and host-admitted effects, without a host process."""

from contextlib import contextmanager
from dataclasses import dataclass
import os
from pathlib import Path
import socket
import struct
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import create_autospec, patch

from mas_host import Context, EffectResult, EffectUncertain, FramedChannel, HostError, Request, serve
from mas_host.codec import dumps_object, loads_object
from mas_host.worker import serve_module


def read_exact(sock, length):
    result = bytearray()
    while len(result) < length:
        chunk = sock.recv(length - len(result))
        if not chunk:
            raise EOFError("peer closed an incomplete frame")
        result.extend(chunk)
    return bytes(result)


def read_frame(sock):
    length = struct.unpack(">I", read_exact(sock, 4))[0]
    return loads_object(read_exact(sock, length))


def write_frame(sock, value):
    raw = dumps_object(value)
    sock.sendall(struct.pack(">I", len(raw)) + raw)


@contextmanager
def host_session(testcase, script):
    worker, host = socket.socketpair()
    worker.settimeout(3)
    host.settimeout(3)
    errors = []

    def run_host():
        try:
            script(host)
        except BaseException as error:
            errors.append(error)
        finally:
            host.close()

    thread = threading.Thread(target=run_host, daemon=True)
    thread.start()
    try:
        yield Context(FramedChannel(worker), "planner", "native-invocation")
    finally:
        worker.close()
        thread.join(4)
        testcase.assertFalse(thread.is_alive(), "fake host did not finish")
        if errors:
            raise errors[0]


@dataclass(frozen=True)
class Work:
    quantity: int


@dataclass(frozen=True)
class Answer:
    total: int


class FramingTests(unittest.TestCase):
    def test_expired_original_deadline_stops_channel_io(self):
        worker, host = socket.socketpair()
        try:
            channel = FramedChannel(worker, deadline_ms=1)
            with self.assertRaises(TimeoutError):
                channel.send({"type": "request"})
            with self.assertRaises(TimeoutError):
                channel.receive()
            host.setblocking(False)
            with self.assertRaises(BlockingIOError):
                host.recv(1)
        finally:
            worker.close()
            host.close()

    def test_receives_fragmented_big_endian_frames(self):
        worker, host = socket.socketpair()
        worker.settimeout(3)
        value = {"type": "response", "ok": True, "output": {"n": 2**90}}
        raw = dumps_object(value)
        frame = struct.pack(">I", len(raw)) + raw

        def send_fragments():
            try:
                for part in (frame[:1], frame[1:3], frame[3:8], frame[8:]):
                    host.sendall(part)
            finally:
                host.close()

        thread = threading.Thread(target=send_fragments, daemon=True)
        thread.start()
        try:
            self.assertEqual(FramedChannel(worker).receive(), value)
        finally:
            worker.close()
            thread.join(3)

    def test_sends_one_big_endian_object_frame(self):
        worker, host = socket.socketpair()
        worker.settimeout(3)
        host.settimeout(3)
        try:
            FramedChannel(worker).send({"integer": 2**90})
            self.assertEqual(read_frame(host), {"integer": 2**90})
        finally:
            worker.close()
            host.close()

    def test_invalid_length_and_truncated_payload_are_rejected(self):
        for frame in (struct.pack(">I", 0), struct.pack(">I", 65537), struct.pack(">I", 8) + b"{}"):
            with self.subTest(frame=frame):
                worker, host = socket.socketpair()
                worker.settimeout(3)
                host.sendall(frame)
                host.close()
                try:
                    with self.assertRaises((ValueError, HostError, EOFError, OSError)):
                        FramedChannel(worker).receive()
                finally:
                    worker.close()

    def test_frame_reader_enforces_strict_json(self):
        worker, host = socket.socketpair()
        raw = b'{"duplicate":1,"duplicate":2}'
        host.sendall(struct.pack(">I", len(raw)) + raw)
        host.close()
        try:
            with self.assertRaises((ValueError, HostError)):
                FramedChannel(worker).receive()
        finally:
            worker.close()


class ContextTests(unittest.TestCase):
    def test_invalid_step_keys_fail_before_any_channel_io(self):
        for key in ("", "x" * 129, "\u00e9" * 65, "\u0000", "\u0085"):
            for operation in ("request", "effect", "parallel"):
                with self.subTest(key=repr(key), operation=operation):
                    channel = create_autospec(FramedChannel, instance=True)
                    ctx = Context(channel, "planner", "native-invocation")

                    def callback(invocation):
                        self.fail("invalid effect key executed callback")

                    with self.assertRaises((TypeError, ValueError)):
                        if operation == "request":
                            ctx.request(key, "worker", {})
                        elif operation == "effect":
                            ctx.effect(key, {}, callback)
                        else:
                            ctx.parallel([Request(key, "worker", {})])
                    channel.send.assert_not_called()
                    channel.receive.assert_not_called()

    def test_parallel_cardinality_is_validated_before_any_channel_io(self):
        for count in (0, 9):
            with self.subTest(count=count):
                channel = create_autospec(FramedChannel, instance=True)
                ctx = Context(channel, "planner", "native-invocation")
                requests = [Request(f"request-{index}", "worker", {}) for index in range(count)]
                with self.assertRaises((TypeError, ValueError)):
                    ctx.parallel(requests)
                channel.send.assert_not_called()
                channel.receive.assert_not_called()

    def test_parallel_accepts_eight_requests(self):
        requests = [Request(f"request-{index}", "worker", {"index": index}) for index in range(8)]
        outputs = [{"index": index} for index in range(8)]

        def script(host):
            self.assertEqual(read_frame(host), {
                "type": "parallel",
                "requests": [{"key": request.key, "target": request.target, "input": request.input} for request in requests],
            })
            write_frame(host, {"type": "response", "ok": True, "outputs": outputs})

        with host_session(self, script) as ctx:
            self.assertEqual(ctx.parallel(requests), outputs)

    def test_step_key_accepts_exactly_128_utf8_bytes(self):
        key = "\u00e9" * 64
        self.assertEqual(len(key.encode("utf-8")), 128)

        def script(host):
            self.assertEqual(read_frame(host), {"type": "request", "key": key, "target": "worker", "input": {}})
            write_frame(host, {"type": "response", "ok": True, "output": {"accepted": True}})

        with host_session(self, script) as ctx:
            self.assertEqual(ctx.request(key, "worker", {}), {"accepted": True})

    def test_request_encodes_application_dataclass_and_decodes_result(self):
        def script(host):
            self.assertEqual(read_frame(host), {"type": "request", "key": "work", "target": "counter", "input": {"quantity": 4}})
            write_frame(host, {"type": "response", "ok": True, "output": {"total": 8}})

        with host_session(self, script) as ctx:
            self.assertEqual(ctx.actor, "planner")
            self.assertEqual(ctx.invocation_id, "native-invocation")
            self.assertEqual(ctx.request("work", "counter", Work(4), output_type=Answer), Answer(8))

    def test_parallel_preserves_request_and_result_order(self):
        def script(host):
            self.assertEqual(read_frame(host), {
                "type": "parallel",
                "requests": [
                    {"key": "b", "target": "worker-b", "input": {"quantity": 2}},
                    {"key": "a", "target": "worker-a", "input": {"quantity": 1}},
                ],
            })
            write_frame(host, {"type": "response", "ok": True, "outputs": [{"total": 4}, {"total": 2}]})

        with host_session(self, script) as ctx:
            self.assertEqual(ctx.parallel([Request("b", "worker-b", Work(2)), Request("a", "worker-a", Work(1))]), [{"total": 4}, {"total": 2}])

    def test_parallel_rejects_missing_result(self):
        def script(host):
            read_frame(host)
            write_frame(host, {"type": "response", "ok": True, "outputs": []})

        with host_session(self, script) as ctx:
            with self.assertRaises((ValueError, HostError)):
                ctx.parallel([Request("one", "worker", {})])

    def test_host_rejection_is_reported_without_retry(self):
        calls = []

        def script(host):
            calls.append(read_frame(host))
            write_frame(host, {"type": "response", "ok": False, "error": {"code": "topology_denied"}})

        with host_session(self, script) as ctx:
            with self.assertRaises(HostError):
                ctx.request("forbidden", "outside-topology", {})
        self.assertEqual(len(calls), 1)

    def test_cached_effect_never_executes_callback(self):
        def script(host):
            self.assertEqual(read_frame(host), {"type": "effect_begin", "key": "provider", "descriptor": {"model": "fixed-model"}})
            write_frame(host, {"type": "effect", "state": "cached", "output": {"answer": "recorded"}})

        def callback(invocation):
            self.fail("cached effect executed application callback")

        with host_session(self, script) as ctx:
            self.assertEqual(ctx.effect("provider", {"model": "fixed-model"}, callback), {"answer": "recorded"})

    def test_rejected_effect_admission_never_executes_callback(self):
        def script(host):
            self.assertEqual(read_frame(host)["type"], "effect_begin")
            write_frame(host, {"type": "response", "ok": False, "error": {"code": "budget_exhausted"}})

        def callback(invocation):
            self.fail("rejected effect admission executed callback")

        with host_session(self, script) as ctx:
            with self.assertRaises(HostError):
                ctx.effect("provider", {}, callback)

    def test_admitted_effect_uses_stable_invocation_and_completes_with_evidence(self):
        callback_calls = []

        def script(host):
            self.assertEqual(read_frame(host), {"type": "effect_begin", "key": "write", "descriptor": {"destination": "records"}})
            write_frame(host, {"type": "effect", "state": "execute", "invocation_id": "stable-effect-id"})
            self.assertEqual(read_frame(host), {"type": "effect_complete", "key": "write", "output": {"total": 5}, "evidence": {"receipt": "external-123"}})
            write_frame(host, {"type": "response", "ok": True, "output": {}})

        def callback(invocation):
            callback_calls.append(invocation)
            return EffectResult(Answer(5), {"receipt": "external-123"})

        with host_session(self, script) as ctx:
            self.assertEqual(ctx.effect("write", {"destination": "records"}, callback), {"total": 5})
        self.assertEqual(callback_calls, ["stable-effect-id"])

    def test_governed_completion_is_authoritative_and_replay_never_reexecutes(self):
        fixture = loads_object((Path(__file__).parents[2] / "completion-conformance.json").read_bytes())
        for case in fixture["cases"]:
            with self.subTest(case=case["name"]):
                calls = []
                cached = case.get("output", {"redacted": True})

                def script(host):
                    self.assertEqual(read_frame(host)["type"], "effect_begin")
                    write_frame(host, {"type": "effect", "state": "execute", "invocation_id": "effect-1"})
                    self.assertEqual(read_frame(host)["output"], {"private": "private-local-output"})
                    write_frame(host, {"type": "response", "ok": True, "completion": case["completion"]})
                    self.assertEqual(read_frame(host)["type"], "effect_begin")
                    write_frame(host, {"type": "effect", "state": "cached", "output": cached})

                def callback(identity):
                    calls.append(identity)
                    return EffectResult({"private": "private-local-output"}, {"receipt": "one"})

                with host_session(self, script) as ctx:
                    if case["valid"]:
                        self.assertEqual(ctx.effect("write", {}, callback), cached)
                    else:
                        with self.assertRaisesRegex(HostError, "^invalid governed effect completion$") as error:
                            ctx.effect("write", {}, callback)
                        self.assertNotIsInstance(error.exception, EffectUncertain)
                    # The acknowledged settlement remains known, including when
                    # projection validation failed. The host replay skips work.
                    self.assertEqual(ctx.effect("write", {}, callback), cached)
                self.assertEqual(calls, ["effect-1"])

    def test_lost_completion_acknowledgement_is_uncertain(self):
        completed = []
        callback_calls = []

        def script(host):
            self.assertEqual(read_frame(host)["type"], "effect_begin")
            write_frame(host, {"type": "effect", "state": "execute", "invocation_id": "stable-effect-id"})
            completed.append(read_frame(host))
            # The external work finished, but the host's durable completion
            # cannot be inferred when the acknowledgement is lost.

        def callback(invocation):
            callback_calls.append(invocation)
            return EffectResult({"done": True}, {"receipt": "external-123"})

        with host_session(self, script) as ctx:
            with self.assertRaises(EffectUncertain):
                ctx.effect("write", {}, callback)
            with self.assertRaises(EffectUncertain):
                ctx.request("after-uncertain", "worker", {})
        self.assertEqual(callback_calls, ["stable-effect-id"])
        self.assertEqual(completed[0]["type"], "effect_complete")

    def test_lost_admission_acknowledgement_blocks_requests_without_callback(self):
        admitted = []
        callback_calls = []

        def script(host):
            admitted.append(read_frame(host))
            # The host may have durably admitted this effect before its socket
            # closed. A missing acknowledgement cannot authorise execution.

        def callback(invocation):
            callback_calls.append(invocation)
            return EffectResult({"done": True}, {"receipt": "external-123"})

        with host_session(self, script) as ctx:
            with self.assertRaises(EffectUncertain):
                ctx.effect("write", {}, callback)
            with self.assertRaises(EffectUncertain):
                ctx.request("after-uncertain", "worker", {})
        self.assertEqual(admitted, [{"type": "effect_begin", "key": "write", "descriptor": {}}])
        self.assertEqual(callback_calls, [])

    def test_callback_failure_after_admission_is_uncertain_and_not_repeated(self):
        calls = []

        def script(host):
            self.assertEqual(read_frame(host)["type"], "effect_begin")
            write_frame(host, {"type": "effect", "state": "execute", "invocation_id": "stable-effect-id"})
            # An uncertain Context must stop all further exchanges. A durable
            # host can reconcile the admission before a fresh invocation.
            self.assertEqual(host.recv(1), b"")

        def callback(invocation):
            calls.append(invocation)
            raise OSError("connection lost after sending")

        with host_session(self, script) as ctx:
            with self.assertRaises(EffectUncertain):
                ctx.effect("write", {}, callback)
            with self.assertRaises((EffectUncertain, HostError)):
                ctx.effect("write", {}, callback)
        self.assertEqual(calls, ["stable-effect-id"])

    def test_invalid_completion_evidence_keeps_admitted_effect_uncertain(self):
        def script(host):
            self.assertEqual(read_frame(host)["type"], "effect_begin")
            write_frame(host, {"type": "effect", "state": "execute", "invocation_id": "stable-effect-id"})
            # Invalid evidence cannot be reported as a successful completion.
            self.assertEqual(host.recv(1), b"")

        with host_session(self, script) as ctx:
            with self.assertRaises(EffectUncertain):
                ctx.effect("write", {}, lambda invocation: EffectResult({"done": True}, {}))


class ServeTests(unittest.TestCase):
    def exchange_with_worker(self, handler_name, handler, start, entrypoint=serve):
        worker, host = socket.socketpair()
        host.settimeout(3)
        inherited_fd = os.dup(worker.fileno())
        worker.close()
        errors = []

        def run_worker():
            try:
                entrypoint(handler_name, handler)
            except BaseException as error:
                errors.append(error)

        try:
            with patch.dict(os.environ, {"MAS_WORKER_CHANNEL_FD": str(inherited_fd)}):
                thread = threading.Thread(target=run_worker, daemon=True)
                thread.start()
                try:
                    write_frame(host, start)
                    response = read_frame(host)
                finally:
                    host.close()
                    thread.join(4)
                    self.assertFalse(thread.is_alive(), "worker did not finish")
            if errors:
                raise errors[0]
            return response
        finally:
            host.close()
            # serve owns the inherited descriptor after wrapping it. Close it
            # only when entry-point validation failed before adoption.
            try:
                os.close(inherited_fd)
            except OSError:
                pass

    def start(self, handler="count"):
        return {
            "type": "start",
            "version": "mas-native/1",
            "actor": "worker",
            "handler": handler,
            "input": {"quantity": 3},
            "invocation_id": "native-123",
            "deadline_ms": 0,
        }

    def test_serves_typed_handler_on_private_socket(self):
        observed = []

        def handler(ctx, value: Work) -> Answer:
            observed.append((ctx.actor, ctx.invocation_id))
            return Answer(value.quantity * 2)

        self.assertEqual(self.exchange_with_worker("count", handler, self.start()), {"type": "complete", "output": {"total": 6}})
        self.assertEqual(observed, [("worker", "native-123")])

    def test_handler_exception_is_an_error_frame(self):
        def handler(ctx, value: Work) -> Answer:
            raise ValueError("application rejected quantity")

        response = self.exchange_with_worker("count", handler, self.start())
        self.assertEqual(response["type"], "error")
        self.assertIn("application rejected quantity", response["message"])

    def test_unknown_handler_cannot_execute_application(self):
        calls = []

        def handler(ctx, value: Work) -> Answer:
            calls.append(value)
            return Answer(1)

        response = self.exchange_with_worker("count", handler, self.start("unknown"))
        self.assertEqual(response["type"], "error")
        self.assertIn("handler", response["message"])
        self.assertEqual(calls, [])

    def test_module_import_sees_private_socket_with_inheritance_revoked(self):
        inherited = []

        def handler(ctx, value: Work) -> Answer:
            return Answer(value.quantity)

        def load(module_name):
            self.assertEqual(module_name, "application_worker")
            descriptor = int(os.environ["MAS_WORKER_CHANNEL_FD"])
            inherited.append(os.get_inheritable(descriptor))
            return SimpleNamespace(count=handler)

        # Start with an inheritable duplicate in exchange_with_worker; the
        # entry point must revoke inheritance before importing application code.
        original_duplicate = os.dup

        def inheritable_duplicate(descriptor):
            duplicated = original_duplicate(descriptor)
            os.set_inheritable(duplicated, True)
            return duplicated

        with patch("os.dup", side_effect=inheritable_duplicate), patch("mas_host.worker.importlib.import_module", side_effect=load):
            result = self.exchange_with_worker("count", "application_worker", self.start(), entrypoint=serve_module)
        self.assertEqual(result, {"type": "complete", "output": {"total": 3}})
        self.assertEqual(inherited, [False])


if __name__ == "__main__":
    unittest.main()

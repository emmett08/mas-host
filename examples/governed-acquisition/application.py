"""Observe one synthetic source and exercise host-owned exposure restrictions."""
from pathlib import Path

from mas_host import Context, EffectResult, HostError


PRIVATE = "governed-acquisition-original-person@example.invalid"
POINT = {"version": "governed-deployment-point/1", "cluster_uid": "fixture-cluster-001",
         "namespace_uid": "fixture-namespace-001", "namespace": "demo", "kind": "Deployment",
         "name": "web", "uid": "fixture-deployment-001", "resource_version": "17",
         "generation": 4, "desired_replicas": 3, "ready_replicas": 2,
         "freshness": "NOT_RUN", "assessment": "NOT_RUN"}


def record(name):
    with open(name, "a", encoding="ascii") as stream:
        stream.write("x")


def blocked_action(ctx):
    def execute(_effect_id):
        record("blocked-action-calls")
        return EffectResult({"performed": True}, {"execution": "completed"})
    # A constant descriptor has no caller-selected dependency list. Once this
    # invocation has observed the source, omission cannot remove its exposure.
    return ctx.effect("blocked-action", {"format": "fixture-blocked-action/1"}, execute)


def acquire(ctx):
    def execute(_effect_id):
        record("collector-calls")
        # Only the registered structured projection is retained. The extra
        # original field exercises minimisation and exact-source inspection.
        return EffectResult({**POINT, "discarded_original": PRIVATE}, {"execution": "completed"})
    return ctx.effect("acquire", {"format": "fixture-deployment-read/1"}, execute)


def collector(ctx: Context, value: dict) -> dict:
    if value["mode"] == "baseline":
        assert blocked_action(ctx) == {"performed": True}
        assert ctx.request("blocked-peer", "blocked", {"ready_replicas": 0}) == {"accepted": True}
        return {"acquired": False, "action_denied": False, "peer_denied": False,
                "consumer_accepted": False}

    point = acquire(ctx)
    assert point == POINT
    action_denied = peer_denied = False
    try:
        blocked_action(ctx)
    except HostError:
        action_denied = True
    try:
        ctx.request("blocked-peer", "blocked", {"ready_replicas": 0})
    except HostError:
        peer_denied = True
    consumed = ctx.request("permitted-peer", "permitted", {"ready_replicas": point["ready_replicas"]})
    return {"acquired": True, "action_denied": action_denied, "peer_denied": peer_denied,
            "consumer_accepted": consumed == {"accepted": True}}


def permitted(_ctx: Context, value: dict) -> dict:
    assert value == {"ready_replicas": 2}
    record("permitted-peer-calls")
    Path("consumed-ready-replicas").write_text(str(value["ready_replicas"]), encoding="ascii")
    return {"accepted": True}


def blocked(_ctx: Context, _value: dict) -> dict:
    record("blocked-peer-calls")
    return {"accepted": True}

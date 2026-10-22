"""Project one immutable synthetic Deployment observation without live authority."""
import argparse
import json
from pathlib import Path
import re
import sys


def require(condition):
    if not condition:
        raise ValueError("structured-evidence-rejected")


def record(value, fields):
    require(type(value) is dict and set(value) == set(fields))
    return value


def identifier(value, maximum):
    require(type(value) is str and len(value) <= maximum
            and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:/-]*", value) is not None)
    return value


def name(value):
    require(type(value) is str and re.fullmatch(r"[a-z0-9](?:[-a-z0-9]{0,61}[a-z0-9])?", value) is not None)
    return value


def integer(value, minimum, maximum):
    require(type(value) is int and minimum <= value <= maximum)
    return value


def collect(input_value, fixture_value, invocation_value):
    value = record(input_value, ("operationId", "target"))
    operation_id = identifier(value["operationId"], 160)
    invocation_id = identifier(invocation_value, 160)
    target_fields = ("cluster_uid", "namespace_uid", "namespace", "kind", "name", "uid", "resource_version")
    target = record(value["target"], target_fields)
    fixture = record(fixture_value, ("version", "scope", "deployment"))
    require(fixture["version"] == "structured-deployment-fixture/1")
    scope = record(fixture["scope"], ("cluster_uid", "namespace_uid"))
    deployment = record(fixture["deployment"], ("apiVersion", "kind", "metadata", "spec", "status"))
    require(deployment["apiVersion"] == "apps/v1" and deployment["kind"] == "Deployment")
    metadata = record(deployment["metadata"], ("namespace", "name", "uid", "resourceVersion", "generation"))
    spec = record(deployment["spec"], ("replicas",))
    status = record(deployment["status"], ("observedGeneration", "replicas", "updatedReplicas", "readyReplicas", "availableReplicas", "unavailableReplicas"))
    expected = {
        "cluster_uid": identifier(scope["cluster_uid"], 128), "namespace_uid": identifier(scope["namespace_uid"], 128),
        "namespace": name(metadata["namespace"]), "kind": "Deployment", "name": name(metadata["name"]),
        "uid": identifier(metadata["uid"], 128), "resource_version": identifier(metadata["resourceVersion"], 128),
    }
    require(target == expected)
    facts = {
        "generation": integer(metadata["generation"], 1, 2147483647),
        "observed_generation": integer(status["observedGeneration"], 0, 2147483647),
        "desired_replicas": integer(spec["replicas"], 0, 100), "replicas": integer(status["replicas"], 0, 100),
        "updated_replicas": integer(status["updatedReplicas"], 0, 100), "ready_replicas": integer(status["readyReplicas"], 0, 100),
        "available_replicas": integer(status["availableReplicas"], 0, 100), "unavailable_replicas": integer(status["unavailableReplicas"], 0, 100),
    }
    return {"version": "structured-deployment-observation/1", "operationId": operation_id,
            "invocation_id": invocation_id, "target": expected, "source": "synthetic-fixture/1",
            "freshness": "NOT_RUN", "assessment": "NOT_RUN",
            "coverage": {"scope": "one-deployment-point", "omissions": ["pods", "images", "events", "logs", "live-freshness"]}, "facts": facts}


def parse(raw):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result)
            result[key] = value
        return result
    require(len(raw) <= 8192)
    return json.loads(raw, object_pairs_hook=unique,
                      parse_float=lambda _: (_ for _ in ()).throw(ValueError()),
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))


def main():
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--fixture", required=True)
    parser.add_argument("--invocation", required=True)
    args = parser.parse_args()
    with Path(args.fixture).open("rb") as source:
        fixture = parse(source.read(8193))
    value = parse(sys.stdin.buffer.read(8193))
    encoded = json.dumps(collect(value, fixture, args.invocation), separators=(",", ":"))
    require(len(encoded.encode()) <= 8192)
    print(encoded)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, TypeError, KeyError, OSError, UnicodeError, RecursionError):
        print("structured-evidence-rejected", file=sys.stderr)
        sys.exit(2)

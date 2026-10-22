#!/usr/bin/env python3
"""Render a bounded single-owner deployment through the maintained host CLI.

The resulting Kubernetes List is JSON, accepted directly by kubectl. This tool
never contacts Kubernetes and never reads or embeds credential values.
"""
from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import pathlib
import re
import subprocess
import sys
from urllib.parse import urlsplit

VERSION = "mas-host-deployment/1"
LABEL = re.compile(r"[a-z][a-z0-9-]{0,38}[a-z0-9]$|[a-z]$")
IMAGE = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9._:/-]*@sha256:[0-9a-f]{64}$")
ENV = re.compile(r"[A-Z][A-Z0-9_]{0,63}$")
REQUIRED = {
    "version", "namespace", "name", "profile", "host_image", "proxy_image",
    "public_url", "host_binary", "bundle_path", "configuration_secret", "commitment_secret",
    "tls_secret", "state_claim", "export_claim", "lifecycle_claim", "ingress_namespace", "exports",
    "ingress_pod_labels", "credential_references", "provider_endpoints", "collector",
}


def load_json(raw: bytes | str) -> dict:
    def fields(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON field")
            result[key] = value
        return result

    def nonfinite(_value):
        raise ValueError("non-finite JSON number")

    return json.loads(raw, object_pairs_hook=fields, parse_constant=nonfinite)


def check_name(value: object) -> str:
    if not isinstance(value, str) or not LABEL.fullmatch(value):
        raise ValueError("invalid Kubernetes name")
    return value


def image(value: object) -> str:
    if not isinstance(value, str) or not IMAGE.fullmatch(value):
        raise ValueError("images require an explicit sha256 digest")
    if value.endswith("0" * 64):
        raise ValueError("replace example image digest placeholders before rendering")
    return value


def absolute_path(value: object) -> str:
    if not isinstance(value, str) or len(value) > 256:
        raise ValueError("invalid image path")
    path = pathlib.PurePosixPath(value)
    if not path.is_absolute() or ".." in path.parts or str(path) != value:
        raise ValueError("image path must be absolute and canonical")
    return value


def endpoint(value: object) -> dict:
    if not isinstance(value, dict) or set(value) != {"cidr", "port"}:
        raise ValueError("endpoint requires cidr and port")
    if not isinstance(value["cidr"], str):
        raise ValueError("endpoint cidr must be a string")
    address = ipaddress.ip_network(value["cidr"], strict=True)
    if address.prefixlen != address.max_prefixlen:
        raise ValueError("egress endpoints must identify one IP address")
    if type(value["port"]) is not int or not 1 <= value["port"] <= 65535:
        raise ValueError("invalid endpoint port")
    return {"to": [{"ipBlock": {"cidr": str(address)}}],
            "ports": [{"protocol": "TCP", "port": value["port"]}]}


def validate(config: dict) -> dict:
    if not isinstance(config, dict) or set(config) != REQUIRED or config["version"] != VERSION:
        raise ValueError("unsupported deployment shape or version")
    for key in ("namespace", "name", "configuration_secret", "commitment_secret", "tls_secret", "state_claim", "export_claim", "lifecycle_claim", "ingress_namespace"):
        check_name(config[key])
    if len({config["state_claim"], config["export_claim"], config["lifecycle_claim"]}) != 3:
        raise ValueError("execution, exports and lifecycle require separate volume claims")
    if config["profile"] not in ("trusted-local", "managed-cluster"):
        raise ValueError("shared-enterprise requires a separately qualified deployment")
    image(config["host_image"])
    image(config["proxy_image"])
    absolute_path(config["host_binary"])
    absolute_path(config["bundle_path"])
    if (not isinstance(config["public_url"], str) or len(config["public_url"]) > 2000
            or any(char.isspace() or ord(char) < 32 for char in config["public_url"])):
        raise ValueError("invalid public_url")
    url = urlsplit(config["public_url"])
    if (url.scheme != "https" or not url.hostname or url.username or url.password
            or url.path != "/a2a" or url.query or url.fragment):
        raise ValueError("public_url must be an HTTPS /a2a endpoint")
    exports = config["exports"]
    if not isinstance(exports, list) or not 1 <= len(exports) <= 16:
        raise ValueError("one to sixteen explicit exports are required")
    actors = set()
    for export in exports:
        if not isinstance(export, dict) or set(export) != {"agent", "name", "description", "public"}:
            raise ValueError("unsupported export shape")
        check_name(export["agent"])
        if export["agent"] in actors or type(export["public"]) is not bool:
            raise ValueError("invalid or repeated export")
        actors.add(export["agent"])
        for field in ("name", "description"):
            if not isinstance(export[field], str) or not 1 <= len(export[field].encode()) <= 512:
                raise ValueError("export metadata exceeds byte bound")
    labels = config["ingress_pod_labels"]
    if not isinstance(labels, dict) or not 1 <= len(labels) <= 8:
        raise ValueError("ingress requires an exact Pod label selector")
    for key, value in labels.items():
        if not isinstance(key, str) or not re.fullmatch(r"[a-zA-Z0-9./_-]{1,128}", key):
            raise ValueError("invalid ingress label")
        if not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9._-]{1,63}", value):
            raise ValueError("invalid ingress label value")
    credentials = config["credential_references"]
    if not isinstance(credentials, list) or len(credentials) > 16:
        raise ValueError("too many credential references")
    names = set()
    for ref in credentials:
        if not isinstance(ref, dict) or set(ref) != {"name", "secret", "key"}:
            raise ValueError("credentials must be explicit Secret references")
        if not isinstance(ref["name"], str) or not ENV.fullmatch(ref["name"]) or ref["name"] in names:
            raise ValueError("invalid or repeated credential name")
        if ref["name"].startswith(("KUBERNETES_", "LD_", "PYTHON", "NODE_", "RUST_", "NEHEMIAH_")) or ref["name"] in {"PATH", "HOME"}:
            raise ValueError("runtime and cluster environment overrides are forbidden")
        names.add(ref["name"])
        check_name(ref["secret"])
        if not isinstance(ref["key"], str) or not re.fullmatch(r"[A-Za-z0-9._-]{1,128}", ref["key"]):
            raise ValueError("invalid Secret key")
    endpoints = config["provider_endpoints"]
    if not isinstance(endpoints, list) or len(endpoints) > 16:
        raise ValueError("too many provider endpoints")
    for item in endpoints:
        endpoint(item)
    collector = config["collector"]
    if collector is not None:
        if not isinstance(collector, dict) or set(collector) != {"image", "command", "api_endpoint", "observation_namespace", "operation_id"}:
            raise ValueError("unsupported collector configuration")
        image(collector["image"])
        check_name(collector["observation_namespace"])
        if not isinstance(collector["operation_id"], str) or not re.fullmatch(r"[a-zA-Z0-9_-]{1,128}", collector["operation_id"]):
            raise ValueError("collector requires a stable original operation identity")
        endpoint(collector["api_endpoint"])
        command = collector["command"]
        if (not isinstance(command, list) or not 1 <= len(command) <= 32
                or any(not isinstance(arg, str) or len(arg.encode()) > 1024 or "\x00" in arg for arg in command)):
            raise ValueError("collector command must be bounded fixed argv")
        absolute_path(command[0])
    return config


def profile(config: dict) -> dict:
    capabilities = ["trusted-local-execution", "local-effect-replay"]
    if config["profile"] == "managed-cluster":
        # The maintained host supplies its own mandatory managed requirements.
        # The deployment must not invent support by supplying a shorter list.
        capabilities = []
    return {"version": "mas-host-profile/1", "profile": config["profile"],
            "required_capabilities": capabilities, "worker_isolation": False}


def preflight(config: dict, host: str, profile_path: pathlib.Path) -> None:
    profile_path.write_text(json.dumps(profile(config), sort_keys=True) + "\n")
    completed = subprocess.run([host, "check-profile", "--profile", str(profile_path)],
                               capture_output=True, timeout=30, check=False)
    if completed.returncode:
        # Do not copy arbitrary process diagnostics into an export.
        raise ValueError("host profile preflight rejected deployment requirements")
    try:
        result = load_json(completed.stdout)
    except (ValueError, UnicodeDecodeError) as error:
        raise ValueError("invalid host profile response") from error
    if (not isinstance(result, dict) or result.get("profile") != config["profile"]
            or result.get("version") != "mas-host-profile-check/1" or result.get("accepted") is not True):
        raise ValueError("host profile identity did not match")


def render(config: dict) -> dict:
    validate(config)
    namespace, name = config["namespace"], config["name"]
    binding = hashlib.sha256(json.dumps(config, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    base_labels = {"app.kubernetes.io/part-of": "nehemiah", "app.kubernetes.io/instance": name}
    documents = []

    def add(kind, resource_name, **body):
        api = {"Deployment": "apps/v1", "Job": "batch/v1", "Role": "rbac.authorization.k8s.io/v1",
               "RoleBinding": "rbac.authorization.k8s.io/v1", "NetworkPolicy": "networking.k8s.io/v1"}.get(kind, "v1")
        obj = {"apiVersion": api, "kind": kind,
               "metadata": {"name": resource_name, "namespace": namespace, "labels": base_labels.copy()}, **body}
        documents.append(obj)
        return obj

    add("ServiceAccount", name + "-host", automountServiceAccountToken=False)
    # No RoleBinding is granted to the host or to the separately reserved action
    # identity. Applications must qualify their mutation adapter independently.
    add("ServiceAccount", name + "-action", automountServiceAccountToken=False)
    add("ConfigMap", name + "-deployment", data={"profile.json": json.dumps(profile(config), sort_keys=True)})
    service = {"version": "mas-host-a2a/2", "listen": "127.0.0.1:8765", "public_url": config["public_url"],
               "name": name, "authentication_file": "/etc/nehemiah/authentication.json",
               "governance_file": "/etc/nehemiah/governance.json", "exports": config["exports"],
               "max_active": 1, "max_tasks": 1000}
    add("Secret", name + "-service", type="Opaque", stringData={"service.json": json.dumps(service, sort_keys=True)})
    proxy_config = """pid /tmp/nginx.pid;
events { worker_connections 128; }
http {
  access_log off;
  error_log /dev/stderr crit;
  client_body_temp_path /tmp/client;
  proxy_temp_path /tmp/proxy;
  server {
    listen 8443 ssl;
    ssl_certificate /tls/tls.crt;
    ssl_certificate_key /tls/tls.key;
    ssl_protocols TLSv1.2 TLSv1.3;
    client_max_body_size 128k;
    client_body_timeout 10s;
    location = /a2a {
      proxy_pass http://127.0.0.1:8765;
      proxy_request_buffering off;
      proxy_buffering off;
      proxy_read_timeout 30s;
      proxy_set_header Host $host;
    }
    location = /.well-known/agent-card.json {
      proxy_pass http://127.0.0.1:8765;
      proxy_buffering off;
    }
    location / { return 404; }
  }
}
"""
    add("ConfigMap", name + "-proxy", data={"nginx.conf": proxy_config})
    security = {"allowPrivilegeEscalation": False, "readOnlyRootFilesystem": True,
                "capabilities": {"drop": ["ALL"]}}
    resources = {"requests": {"cpu": "100m", "memory": "128Mi"},
                 "limits": {"cpu": "1", "memory": "512Mi", "ephemeral-storage": "64Mi"}}
    pod_security = {"runAsNonRoot": True, "runAsUser": 65532, "runAsGroup": 65532,
                    "fsGroup": 65532, "seccompProfile": {"type": "RuntimeDefault"}}
    host_mounts = [{"name": "state", "mountPath": "/state"},
                   {"name": "lifecycle", "mountPath": "/control"},
                   {"name": "exports", "mountPath": "/exports", "readOnly": True},
                   {"name": "configuration", "mountPath": "/etc/nehemiah", "readOnly": True},
                   {"name": "private-keys", "mountPath": "/private-keys", "readOnly": True},
                   {"name": "service", "mountPath": "/etc/service", "readOnly": True},
                   {"name": "deployment", "mountPath": "/etc/deployment", "readOnly": True},
                   {"name": "host-tmp", "mountPath": "/tmp"}]
    host = {"name": "host", "image": config["host_image"], "imagePullPolicy": "IfNotPresent",
            "command": [config["host_binary"]],
            "args": ["serve-a2a", "--bundle", config["bundle_path"], "--state", "/state", "--config", "/etc/service/service.json"],
            "securityContext": security, "resources": resources, "volumeMounts": host_mounts,
            "env": [{"name": ref["name"], "valueFrom": {"secretKeyRef": {"name": ref["secret"], "key": ref["key"]}}}
                    for ref in config["credential_references"]]}
    proxy = {"name": "tls", "image": config["proxy_image"], "command": ["/usr/sbin/nginx"],
             "args": ["-g", "daemon off;", "-c", "/etc/proxy/nginx.conf"],
             "ports": [{"name": "https", "containerPort": 8443}], "securityContext": security,
             "resources": {"requests": {"cpu": "25m", "memory": "32Mi"},
                           "limits": {"cpu": "250m", "memory": "64Mi", "ephemeral-storage": "32Mi"}},
             "volumeMounts": [{"name": "proxy", "mountPath": "/etc/proxy", "readOnly": True},
                              {"name": "tls", "mountPath": "/tls", "readOnly": True},
                              {"name": "proxy-tmp", "mountPath": "/tmp"}],
             "readinessProbe": {"httpGet": {"scheme": "HTTPS", "port": "https", "path": "/.well-known/agent-card.json"},
                                "periodSeconds": 5, "timeoutSeconds": 2}}
    labels = {**base_labels, "nehemiah-role": "host"}
    add("Deployment", name, spec={"replicas": 1, "strategy": {"type": "Recreate"},
        "selector": {"matchLabels": labels}, "template": {"metadata": {"labels": labels,
            "annotations": {"nehemiah/configuration-sha256": binding, "nehemiah/execution-profile": config["profile"]}},
        "spec": {"serviceAccountName": name + "-host", "automountServiceAccountToken": False,
            "securityContext": pod_security, "terminationGracePeriodSeconds": 40,
            "initContainers": [{"name": "profile-preflight", "image": config["host_image"],
                "command": [config["host_binary"], "check-profile", "--profile", "/etc/deployment/profile.json"],
                "securityContext": security, "resources": resources,
                "volumeMounts": [{"name": "deployment", "mountPath": "/etc/deployment", "readOnly": True}]},
                {"name": "private-key", "image": config["host_image"],
                 "command": ["/bin/sh", "-ec", "umask 077; /bin/cp /key-source/commitment.key /private-keys/commitment.key; /bin/chmod 600 /private-keys/commitment.key"],
                 "securityContext": security, "resources": resources,
                 "volumeMounts": [{"name": "commitment-source", "mountPath": "/key-source", "readOnly": True},
                                  {"name": "private-keys", "mountPath": "/private-keys"}]}],
            "containers": [host, proxy],
            "volumes": [{"name": "state", "persistentVolumeClaim": {"claimName": config["state_claim"]}},
                        {"name": "lifecycle", "persistentVolumeClaim": {"claimName": config["lifecycle_claim"]}},
                        {"name": "exports", "persistentVolumeClaim": {"claimName": config["export_claim"], "readOnly": True}},
                        {"name": "configuration", "secret": {"secretName": config["configuration_secret"], "defaultMode": 288}},
                        {"name": "commitment-source", "secret": {"secretName": config["commitment_secret"], "defaultMode": 288,
                                                                 "items": [{"key": "commitment.key", "path": "commitment.key"}]}},
                        {"name": "private-keys", "emptyDir": {"medium": "Memory", "sizeLimit": "1Mi"}},
                        {"name": "service", "secret": {"secretName": name + "-service", "defaultMode": 288}},
                        {"name": "tls", "secret": {"secretName": config["tls_secret"], "defaultMode": 288}},
                        {"name": "deployment", "configMap": {"name": name + "-deployment"}},
                        {"name": "proxy", "configMap": {"name": name + "-proxy"}},
                        {"name": "host-tmp", "emptyDir": {"medium": "Memory", "sizeLimit": "32Mi"}},
                        {"name": "proxy-tmp", "emptyDir": {"medium": "Memory", "sizeLimit": "16Mi"}}]}}})
    add("Service", name, spec={"type": "ClusterIP", "selector": labels,
        "ports": [{"name": "https", "port": 443, "targetPort": "https"}]})
    add("NetworkPolicy", name + "-host", spec={"podSelector": {"matchLabels": labels},
        "policyTypes": ["Ingress", "Egress"],
        "ingress": [{"from": [{"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": config["ingress_namespace"]}},
                                 "podSelector": {"matchLabels": config["ingress_pod_labels"]}}],
                     "ports": [{"protocol": "TCP", "port": 8443}]}],
        "egress": [endpoint(item) for item in config["provider_endpoints"]]})
    collector = config["collector"]
    if collector:
        collector_name = name + "-collector"
        collector_labels = {**base_labels, "nehemiah-role": "collector"}
        observation_namespace = collector["observation_namespace"]
        # Role lives in the observation namespace; the subject names its separate
        # execution namespace explicitly. Cross-namespace collection stays scoped.
        role = add("Role", collector_name, rules=[{"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get", "list", "watch"]}])
        role["metadata"]["namespace"] = observation_namespace
        binding_obj = add("RoleBinding", collector_name,
            subjects=[{"kind": "ServiceAccount", "name": collector_name, "namespace": namespace}],
            roleRef={"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": collector_name})
        binding_obj["metadata"]["namespace"] = observation_namespace
        add("ServiceAccount", collector_name, automountServiceAccountToken=False)
        add("Job", collector_name, spec={"backoffLimit": 0, "activeDeadlineSeconds": 300,
            "parallelism": 1, "completions": 1, "ttlSecondsAfterFinished": 3600,
            "template": {"metadata": {"labels": collector_labels,
                "annotations": {"nehemiah/configuration-sha256": binding}},
            "spec": {"serviceAccountName": collector_name, "automountServiceAccountToken": False,
                "securityContext": pod_security, "restartPolicy": "Never", "terminationGracePeriodSeconds": 20,
                "containers": [{"name": "collector", "image": collector["image"], "command": collector["command"],
                    "securityContext": security, "resources": resources,
                    "env": [{"name": "NEHEMIAH_OBSERVATION_NAMESPACE", "value": observation_namespace},
                            {"name": "NEHEMIAH_OPERATION_ID", "value": collector["operation_id"]},
                            {"name": "NEHEMIAH_EXPORT_DIRECTORY", "value": "/exports"}],
                    "volumeMounts": [{"name": "exports", "mountPath": "/exports"},
                        {"name": "kube-api", "mountPath": "/var/run/secrets/kubernetes.io/serviceaccount", "readOnly": True},
                        {"name": "tmp", "mountPath": "/tmp"}]}],
                "volumes": [{"name": "exports", "persistentVolumeClaim": {"claimName": config["export_claim"]}},
                    {"name": "tmp", "emptyDir": {"medium": "Memory", "sizeLimit": "32Mi"}},
                    {"name": "kube-api", "projected": {"defaultMode": 288, "sources": [
                        {"serviceAccountToken": {"path": "token", "expirationSeconds": 600}},
                        {"configMap": {"name": "kube-root-ca.crt", "items": [{"key": "ca.crt", "path": "ca.crt"}]}},
                        {"downwardAPI": {"items": [{"path": "namespace", "fieldRef": {"fieldPath": "metadata.namespace"}}]}}]}}]}}})
        add("NetworkPolicy", collector_name, spec={"podSelector": {"matchLabels": collector_labels},
            "policyTypes": ["Ingress", "Egress"], "ingress": [], "egress": [endpoint(collector["api_endpoint"])]})
    return {"apiVersion": "v1", "kind": "List", "items": documents}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("configuration", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--host", help="Maintained mas-host executable for local capability preflight")
    parser.add_argument("--candidate", action="store_true", help="Render for review without claiming a qualified profile")
    args = parser.parse_args()
    try:
        raw = args.configuration.read_bytes()
        if len(raw) > 65536:
            raise ValueError("deployment configuration exceeds byte bound")
        config = validate(load_json(raw))
        args.output.mkdir(parents=True, exist_ok=False)
        profile_path = args.output / "profile.json"
        if args.candidate:
            profile_path.write_text(json.dumps(profile(config), indent=2) + "\n")
        elif args.host:
            preflight(config, args.host, profile_path)
        else:
            raise ValueError("use --host for preflight or --candidate for a review-only render")
        manifest = render(config)
        (args.output / "resources.json").write_text(json.dumps(manifest, indent=2) + "\n")
        (args.output / "qualification.json").write_text(json.dumps({
            "version": "mas-host-deployment-qualification/1", "profile": config["profile"],
            "rendered": True, "host_preflight": "NOT_RUN" if args.candidate else "PASS",
            "image_build": "NOT_RUN", "cluster_rbac": "NOT_RUN", "cluster_egress": "NOT_RUN",
            "volume_lock": "NOT_RUN", "restore": "NOT_RUN", "worker_isolation": "UNSUPPORTED",
            "configuration_sha256": hashlib.sha256(json.dumps(config, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
        }, indent=2) + "\n")
        return 0
    except (OSError, ValueError, TypeError, subprocess.SubprocessError):
        print("deployment preparation failed; configuration or required host capability rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

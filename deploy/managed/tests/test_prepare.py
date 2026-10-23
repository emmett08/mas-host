import json
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import prepare
import qualify


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.config = json.loads((ROOT / "config.example.json").read_text())
        self.config["host_image"] = "registry.example/application@sha256:" + "1" * 64
        self.config["proxy_image"] = "registry.example/proxy@sha256:" + "2" * 64
        self.config["collector"]["image"] = "registry.example/acquire@sha256:" + "3" * 64

    def resources(self):
        return prepare.render(self.config)["items"]

    def find(self, kind, name=None):
        return next(item for item in self.resources() if item["kind"] == kind and (name is None or item["metadata"]["name"] == name))

    def test_actual_host_flags_and_loopback_service(self):
        deployment = self.find("Deployment")
        host, proxy = deployment["spec"]["template"]["spec"]["containers"]
        self.assertEqual(host["args"], ["serve-a2a", "--bundle", self.config["bundle_path"], "--state", "/state", "--config", "/etc/service/service.json"])
        service = json.loads(self.find("Secret")["stringData"]["service.json"])
        self.assertEqual(service["listen"], "127.0.0.1:8765")
        self.assertEqual(service["public_url"], self.config["public_url"])
        self.assertEqual(service["exports"], self.config["exports"])
        self.assertEqual(service["version"], "mas-host-a2a/2")
        self.assertEqual(proxy["ports"][0]["containerPort"], 8443)

    def test_source_publication_requires_explicit_source_read_contract(self):
        self.config["source_publication"] = True
        service = json.loads(self.find("Secret")["stringData"]["service.json"])
        self.assertTrue(service["source_publication"])
        self.config["source_publication"] = False
        service = json.loads(self.find("Secret")["stringData"]["service.json"])
        self.assertFalse(service["source_publication"])
        self.config["source_publication"] = "true"
        with self.assertRaises(ValueError):
            prepare.validate(self.config)
        self.config["source_publication"] = True
        self.config["governance_version"] = "mas-governed-local/1"
        with self.assertRaises(ValueError):
            prepare.validate(self.config)
        del self.config["governance_version"]
        with self.assertRaises(ValueError):
            prepare.validate(self.config)
        del self.config["source_publication"]
        self.assertNotIn("source_publication", json.loads(self.find("Secret")["stringData"]["service.json"]))

    def test_images_must_be_digest_pinned(self):
        for field in ("host_image", "proxy_image"):
            self.config[field] = "registry.example/image:latest"
            with self.assertRaises(ValueError):
                prepare.validate(self.config)
            self.setUp()
        self.config["collector"]["image"] = "registry.example/collector:stable"
        with self.assertRaises(ValueError):
            prepare.validate(self.config)

    def test_credential_names_not_values_and_no_runtime_injection(self):
        ref = {"name": "MODEL_API_KEY", "secret": "model-api", "key": "api-key"}
        self.config["credential_references"] = [ref]
        host = self.find("Deployment")["spec"]["template"]["spec"]["containers"][0]
        self.assertEqual(host["env"], [{"name": "MODEL_API_KEY", "valueFrom": {"secretKeyRef": {"name": "model-api", "key": "api-key"}}}])
        for name in ("PATH", "HOME", "LD_PRELOAD", "PYTHONPATH", "KUBERNETES_SERVICE_HOST"):
            ref["name"] = name
            with self.assertRaises(ValueError):
                prepare.validate(self.config)
        ref["name"] = "MODEL_API_KEY"
        ref["value"] = "plaintext-secret"
        with self.assertRaises(ValueError):
            prepare.validate(self.config)

    def test_all_pods_hardened_no_host_or_wide_storage(self):
        for workload in (self.find("Deployment"), self.find("Job")):
            spec = workload["spec"]["template"]["spec"]
            self.assertFalse(spec["automountServiceAccountToken"])
            self.assertTrue(spec["securityContext"]["runAsNonRoot"])
            self.assertEqual(spec["securityContext"]["seccompProfile"], {"type": "RuntimeDefault"})
            self.assertFalse(set(spec) & {"hostNetwork", "hostPID", "hostIPC"})
            self.assertTrue(all("hostPath" not in volume for volume in spec["volumes"]))
            for container in spec["containers"] + spec.get("initContainers", []):
                self.assertTrue(container["securityContext"]["readOnlyRootFilesystem"])
                self.assertFalse(container["securityContext"]["allowPrivilegeEscalation"])
                self.assertEqual(container["securityContext"]["capabilities"]["drop"], ["ALL"])
                self.assertIn("memory", container["resources"]["limits"])

    def test_collector_read_only_namespace_and_separate_action_identity(self):
        role = self.find("Role")
        self.assertEqual(role["metadata"]["namespace"], "commerce")
        self.assertEqual(role["rules"], [{"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get", "list", "watch"]}])
        role_bindings = [item for item in self.resources() if item["kind"] == "RoleBinding"]
        self.assertEqual(len(role_bindings), 1)
        self.assertEqual(role_bindings[0]["subjects"], [{"kind": "ServiceAccount", "name": "investigation-collector", "namespace": "nehemiah"}])
        self.assertTrue(self.find("ServiceAccount", "investigation-action"))
        collector = self.find("Job")["spec"]
        self.assertEqual(collector["backoffLimit"], 0)
        env = collector["template"]["spec"]["containers"][0]["env"]
        self.assertIn({"name": "NEHEMIAH_OPERATION_ID", "value": "approved-investigation-001"}, env)

    def test_bounded_network_defaults(self):
        host_policy = self.find("NetworkPolicy", "investigation-host")["spec"]
        self.assertEqual(host_policy["egress"], [])
        peer = host_policy["ingress"][0]["from"][0]
        self.assertIn("podSelector", peer)
        self.assertIn("namespaceSelector", peer)
        for cidr in ("0.0.0.0/0", "10.0.0.0/8", "::/0"):
            self.config["provider_endpoints"] = [{"cidr": cidr, "port": 443}]
            with self.assertRaises(ValueError):
                prepare.validate(self.config)

    def test_lifecycle_not_restored_with_execution(self):
        volumes = self.find("Deployment")["spec"]["template"]["spec"]["volumes"]
        claims = {item["name"]: item["persistentVolumeClaim"]["claimName"] for item in volumes if "persistentVolumeClaim" in item}
        self.assertEqual(len(set(claims.values())), 3)
        self.config["lifecycle_claim"] = self.config["state_claim"]
        with self.assertRaises(ValueError):
            prepare.validate(self.config)

    def test_every_execution_change_changes_binding(self):
        def digest():
            return self.find("Deployment")["spec"]["template"]["metadata"]["annotations"]["nehemiah/configuration-sha256"]
        original = digest()
        self.config["credential_references"] = [{"name": "MODEL_API_KEY", "secret": "model-api", "key": "api-key"}]
        self.assertNotEqual(digest(), original)
        self.setUp()
        self.config["collector"]["operation_id"] = "next-approved-operation"
        self.assertNotEqual(digest(), original)

    def test_profile_preflight_cannot_silently_downgrade(self):
        self.config["profile"] = "managed-cluster"
        self.assertEqual(prepare.profile(self.config)["profile"], "managed-cluster")
        deployment = self.find("Deployment")
        init = deployment["spec"]["template"]["spec"]["initContainers"][0]
        self.assertEqual(init["command"][1:], ["check-profile", "--profile", "/etc/deployment/profile.json"])
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "profile.json"
            with patch.object(prepare.subprocess, "run", return_value=type("Result", (), {"returncode": 2, "stdout": b""})()):
                with self.assertRaisesRegex(ValueError, "preflight rejected"):
                    prepare.preflight(self.config, "mas-host", path)
            with patch.object(prepare.subprocess, "run", return_value=type("Result", (), {"returncode": 0, "stdout": b'{"profile":"trusted-local","accepted":true}'})()):
                with self.assertRaisesRegex(ValueError, "identity did not match"):
                    prepare.preflight(self.config, "mas-host", path)

    def test_cluster_api_errors_are_not_successful_rbac_denials(self):
        with patch.object(qualify, "run", return_value=(1, "")):
            result = qualify.qualify(self.config, "kubectl")
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["checks"][0]["observed"], "unavailable")
        self.assertEqual(result["egress_enforcement"], "NOT_RUN")
        self.assertEqual(result["worker_isolation"], "UNSUPPORTED")

    def test_proxy_disables_payload_logs_and_buffering(self):
        text = self.find("ConfigMap", "investigation-proxy")["data"]["nginx.conf"]
        self.assertIn("access_log off;", text)
        self.assertIn("proxy_request_buffering off;", text)
        self.assertIn("proxy_buffering off;", text)

    def test_private_key_uses_init_only_source_and_private_mode(self):
        spec = self.find("Deployment")["spec"]["template"]["spec"]
        host = spec["containers"][0]
        self.assertNotIn("commitment-source", [mount["name"] for mount in host["volumeMounts"]])
        private = next(mount for mount in host["volumeMounts"] if mount["name"] == "private-keys")
        self.assertTrue(private["readOnly"])
        init = next(container for container in spec["initContainers"] if container["name"] == "private-key")
        self.assertIn("umask 077", init["command"][2])
        self.assertIn("chmod 600", init["command"][2])
        self.assertNotIn(self.config["name"], init["command"][2])

    def test_duplicate_json_fields_unknown_contracts_and_nonfinite_reject(self):
        for source in ('{"version":"one","version":"two"}', '{"port":NaN}'):
            with self.assertRaises(ValueError):
                prepare.load_json(source)
        self.config["allow_privileged"] = True
        with self.assertRaises(ValueError):
            prepare.validate(self.config)
        del self.config["allow_privileged"]
        self.config["profile"] = "shared-enterprise"
        with self.assertRaises(ValueError):
            prepare.validate(self.config)


if __name__ == "__main__":
    unittest.main()

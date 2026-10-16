"""Python configuration exports the host's versioned authoring documents."""
from pathlib import Path
import os
import tempfile
import unittest

from mas_host.codec import loads_object
from mas_host.configuration import Agent, Configuration, ExportedConfiguration, Llm, Native, Workflow


class ConfigurationTests(unittest.TestCase):
    def test_export_preserves_exact_host_bindings_and_topology(self):
        config = Configuration(
            agents=[
                Agent("python_worker", Native("python", Path("workers.py"), "assess"), "assess/1"),
                Agent("orchestrator", Workflow(Path("plans/release.json"))),
                Agent("reviewer", Llm(
                    connection="review_provider", model="chosen-model",
                    instructions=Path("prompts/review.md"), output_schema="schemas/result.json",
                    input_schema="schemas/task.json", max_output_tokens=2048,
                    max_turns=3, delegates=("python_worker",),
                )),
            ],
            edges=[("client", "orchestrator"), ("orchestrator", "client")],
            connections={"review_provider": {"kind": "openai", "api_key_env": "REVIEW_API_KEY"}},
            limits={"max_calls": 16},
        )
        with tempfile.TemporaryDirectory() as root:
            files = config.export(Path(root) / "generated")
            self.assertIsInstance(files, ExportedConfiguration)
            self.assertEqual(files.agents, Path(root) / "generated" / "agents.json")
            self.assertEqual(files.topology, Path(root) / "generated" / "topology.json")
            agents = loads_object(files.agents.read_bytes())
            self.assertEqual(agents, {
                "version": "mas-agents/2",
                "agents": [
                    {"name": "python_worker", "operation": "assess/1", "implementation": {
                        "kind": "native", "language": "python", "source": "workers.py", "handler": "assess",
                    }},
                    {"name": "orchestrator", "implementation": {"kind": "workflow", "plan": "plans/release.json"}},
                    {"name": "reviewer", "implementation": {
                        "kind": "llm", "connection": "review_provider", "model": "chosen-model",
                        "instructions": "prompts/review.md", "output_schema": "schemas/result.json",
                        "input_schema": "schemas/task.json", "max_output_tokens": 2048,
                        "max_turns": 3, "delegates": ["python_worker"],
                    }},
                ],
                "connections": {"review_provider": {"kind": "openai", "api_key_env": "REVIEW_API_KEY"}},
                "limits": {"max_calls": 16},
            })
            self.assertEqual(loads_object(files.topology.read_bytes()), {
                "version": "mas-topology/1",
                "edges": [["client", "orchestrator"], ["orchestrator", "client"]],
            })

    def test_optional_fields_remain_omitted_and_paths_are_not_read(self):
        document = Configuration(
            [Agent("reviewer", Llm("provider", "model", "missing.md", "missing.json"))],
            [],
        ).to_dict()
        self.assertEqual(set(document), {"version", "agents"})
        self.assertEqual(set(document["agents"][0]), {"name", "implementation"})
        self.assertEqual(set(document["agents"][0]["implementation"]), {
            "kind", "connection", "model", "instructions", "output_schema",
        })

    def test_exports_are_deterministic_independent_documents(self):
        connections = {"local": {"kind": "mock", "output": {"value": 9007199254740993}}}
        config = Configuration([Agent("worker", Native("python", "worker.py", "run"))],
                               [("client", "worker")], connections=connections)
        document = config.to_dict()
        document["connections"]["local"]["output"]["value"] = 0
        self.assertEqual(connections["local"]["output"]["value"], 9007199254740993)
        with tempfile.TemporaryDirectory() as root:
            first = config.export(Path(root) / "first")
            second = config.export(Path(root) / "second")
            self.assertEqual(first.agents.read_bytes(), second.agents.read_bytes())
            self.assertEqual(first.topology.read_bytes(), second.topology.read_bytes())
            self.assertEqual(loads_object(first.agents.read_bytes())["connections"], connections)
        self.assertEqual(config.edges, [("client", "worker")])

    def test_invalid_python_types_are_rejected_before_writing(self):
        worker = Agent("worker", Native("python", "worker.py", "run"))
        invalid = [
            Configuration([worker], [], connections={"local": {"temperature": 0.5}}),
            Configuration([worker], [], limits={"max_calls": True}),
            Configuration([worker], [("client",)]),
            Configuration([worker], [("client", 7)]),
            Configuration([worker], "client-worker"),
            Configuration([{"name": "worker"}], []),
            Configuration([Agent("worker", {"kind": "native"})], []),
            Configuration([Agent("worker", Llm("local", "m", "p", "s", max_turns=True))], []),
            Configuration([Agent("worker", Llm("local", "m", "p", "s", delegates="peer"))], []),
        ]
        with tempfile.TemporaryDirectory() as root:
            for index, config in enumerate(invalid):
                with self.subTest(index=index), self.assertRaises((TypeError, ValueError)):
                    config.export(Path(root) / str(index))
                self.assertFalse((Path(root) / str(index)).exists())

    def test_export_leaves_domain_validation_to_host(self):
        # A directed edge is exported exactly; the host decides whether the
        # deployment profile requires its reciprocal reply edge.
        config = Configuration([Agent("worker", Native("python", "missing.py", "run"))],
                               [("client", "worker")])
        self.assertEqual(config.topology_dict()["edges"], [["client", "worker"]])

    def test_relative_export_paths_can_be_passed_directly_to_project_client(self):
        config = Configuration([Agent("worker", Native("python", "worker.py", "run"))], [])
        with tempfile.TemporaryDirectory() as root:
            relative = os.path.relpath(Path(root) / "app" / ".mas" / "authored")
            exported = config.export(relative)
            self.assertTrue(exported.agents.is_absolute())
            self.assertEqual(exported.agents, Path(root) / "app" / ".mas" / "authored" / "agents.json")


if __name__ == "__main__":
    unittest.main()

"""Explicit Python authoring for the host's ordinary JSON configuration files.

Export runs only when an application calls :meth:`Configuration.export`. The
host reads the exported documents and owns schema, path and permission checks;
resuming a retained bundle never imports this authoring module or application
configuration code.
"""
from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
import os
from pathlib import Path
from typing import Any, Literal

from .codec import dumps_object, loads_object

PathValue = str | os.PathLike[str]


def _text(value: str, label: str) -> str:
    if type(value) is not str:
        raise TypeError(f"{label} must be a string")
    return value


def _path(value: PathValue, label: str) -> str:
    result = os.fspath(value)
    return _text(result, label)


def _sequence(value: Sequence[Any], label: str) -> Sequence[Any]:
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes, bytearray)):
        raise TypeError(f"{label} must be a sequence")
    return value


def _object(value: dict[str, Any]) -> dict[str, Any]:
    # The round trip both enforces the shared exact JSON profile and returns an
    # independent document, including nested connection settings and limits.
    return loads_object(dumps_object(value))


@dataclass(frozen=True, slots=True)
class Native:
    """A function built using one of the host's native language profiles."""

    language: Literal["go", "rust", "cpp", "python"]
    source: PathValue
    handler: str

    def to_dict(self) -> dict[str, Any]:
        return _object({
            "kind": "native",
            "language": _text(self.language, "language"),
            "source": _path(self.source, "source"),
            "handler": _text(self.handler, "handler"),
        })


@dataclass(frozen=True, slots=True)
class Workflow:
    """A workflow plan file relative to the application project."""

    plan: PathValue

    def to_dict(self) -> dict[str, Any]:
        return _object({"kind": "workflow", "plan": _path(self.plan, "plan")})


@dataclass(frozen=True, slots=True)
class Llm:
    """A maintained LLM binding referring to a named provider connection."""

    connection: str
    model: str
    instructions: PathValue
    output_schema: PathValue
    input_schema: PathValue | None = None
    max_output_tokens: int | None = None
    max_turns: int | None = None
    delegates: Sequence[str] | None = None

    def to_dict(self) -> dict[str, Any]:
        document: dict[str, Any] = {
            "kind": "llm",
            "connection": _text(self.connection, "connection"),
            "model": _text(self.model, "model"),
            "instructions": _path(self.instructions, "instructions"),
            "output_schema": _path(self.output_schema, "output_schema"),
        }
        if self.input_schema is not None:
            document["input_schema"] = _path(self.input_schema, "input_schema")
        for name in ("max_output_tokens", "max_turns"):
            value = getattr(self, name)
            if value is not None:
                if type(value) is not int:
                    raise TypeError(f"{name} must be an exact integer")
                document[name] = value
        if self.delegates is not None:
            document["delegates"] = [
                _text(name, "delegate")
                for name in _sequence(self.delegates, "delegates")
            ]
        return _object(document)


@dataclass(frozen=True, slots=True)
class Agent:
    """An actor identity, its implementation and optional public operation."""

    name: str
    implementation: Native | Workflow | Llm
    operation: str | None = None

    def to_dict(self) -> dict[str, Any]:
        if not isinstance(self.implementation, (Native, Workflow, Llm)):
            raise TypeError("implementation must be Native, Workflow or Llm")
        document = {
            "name": _text(self.name, "agent name"),
            "implementation": self.implementation.to_dict(),
        }
        if self.operation is not None:
            document["operation"] = _text(self.operation, "operation")
        return _object(document)


@dataclass(frozen=True, slots=True)
class ExportedConfiguration:
    """Paths to ordinary files accepted by the host's configuration options."""

    agents: Path
    topology: Path


@dataclass(frozen=True, slots=True)
class Configuration:
    """Author agent bindings and explicit directed topology edges.

    Paths inside bindings remain project-relative; exporting never resolves or
    reads prompts, schemas, sources or plans. Connections use the host's provider
    objects, including credential environment-variable names. The host validates
    provider fields and topology when building its immutable execution bundle.
    """

    agents: Sequence[Agent]
    edges: Sequence[tuple[str, str]]
    connections: dict[str, dict[str, Any]] | None = None
    limits: dict[str, int] | None = None

    def to_dict(self) -> dict[str, Any]:
        records = []
        for agent in _sequence(self.agents, "agents"):
            if not isinstance(agent, Agent):
                raise TypeError("agents must contain Agent instances")
            records.append(agent.to_dict())
        document: dict[str, Any] = {"version": "mas-agents/2", "agents": records}
        if self.connections is not None:
            if type(self.connections) is not dict:
                raise TypeError("connections must be an object")
            for connection in self.connections.values():
                if type(connection) is not dict:
                    raise TypeError("each connection must be an object")
            document["connections"] = self.connections
        if self.limits is not None:
            if type(self.limits) is not dict:
                raise TypeError("limits must be an object")
            if any(type(value) is not int for value in self.limits.values()):
                raise TypeError("limits must contain exact integers")
            document["limits"] = self.limits
        return _object(document)

    def topology_dict(self) -> dict[str, Any]:
        edges = []
        for edge in _sequence(self.edges, "edges"):
            pair = _sequence(edge, "edge")
            if len(pair) != 2:
                raise ValueError("each edge must contain two names")
            edges.append([_text(pair[0], "edge source"), _text(pair[1], "edge target")])
        return _object({"version": "mas-topology/1", "edges": edges})

    def export(self, directory: PathValue) -> ExportedConfiguration:
        """Write deterministic JSON documents to an explicitly chosen directory.

        Both documents are encoded before any file is written. Domain validation
        remains the host's responsibility; no application source is executed by
        the host while reading these files or resuming a run.
        """
        agents = dumps_object(self.to_dict())
        topology = dumps_object(self.topology_dict())
        # CLI override paths are otherwise interpreted relative to --project.
        # Return absolute paths so an export under a relative project directory
        # can be passed straight to Client without duplicating that prefix.
        destination = Path(_path(directory, "export directory")).resolve()
        destination.mkdir(parents=True, exist_ok=True)
        result = ExportedConfiguration(destination / "agents.json", destination / "topology.json")
        result.agents.write_bytes(agents)
        result.topology.write_bytes(topology)
        return result

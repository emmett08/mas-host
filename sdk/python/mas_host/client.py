"""Run typed application input through the host CLI without temporary input files."""
from __future__ import annotations

import os
import subprocess
import uuid
from typing import Any

from .adaptation import decode, encode
from .codec import dumps_object, loads_cli_object, validate_object
from .errors import HostError


class Client:
    """One explicit project/bundle, state directory and resumable run identity."""

    def __init__(self, *, project: str | os.PathLike[str] | None = None,
                 bundle: str | os.PathLike[str] | None = None,
                 state: str | os.PathLike[str] = ".mas/state", run_id: str | None = None,
                 host: str | os.PathLike[str] = "mas-host",
                 sdk_root: str | os.PathLike[str] | None = None,
                 agents: str | os.PathLike[str] | None = None,
                 topology: str | os.PathLike[str] | None = None,
                 connections: str | os.PathLike[str] | None = None):
        if (project is None) == (bundle is None):
            raise ValueError("provide exactly one of project and bundle")
        if bundle is not None and any(value is not None for value in (agents, topology, connections)):
            raise ValueError("configuration overrides require a project")
        self.host = os.fspath(host)
        self.project = os.fspath(project) if project is not None else None
        self.bundle = os.fspath(bundle) if bundle is not None else None
        self.state = os.fspath(state)
        self.run_id = run_id if run_id is not None else f"python-{uuid.uuid4().hex}"
        if type(self.run_id) is not str or not self.run_id:
            raise ValueError("run_id must be a nonempty string")
        self._options = {"--sdk-root": sdk_root, "--agents": agents,
                         "--topology": topology, "--connections": connections}

    def run(self, agent: str, input: Any, *, output_type: Any = None) -> Any:
        """Submit once; the host owns replay and uncertain-effect recovery."""
        if type(agent) is not str or not agent:
            raise ValueError("agent must be a nonempty name")
        raw = dumps_object(encode(input))
        command = [self.host, "run", "--input", "-", "--agent", agent,
                   "--state", self.state, "--run", self.run_id]
        command.extend(["--project", self.project] if self.project is not None
                       else ["--bundle", self.bundle])
        for flag, value in self._options.items():
            if value is not None:
                command.extend([flag, os.fspath(value)])
        try:
            completed = subprocess.run(command, input=raw, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, check=False)
        except OSError as error:
            raise HostError("cannot start MAS host") from error
        if completed.returncode:
            try:
                failure = loads_cli_object(completed.stderr)
                message = failure.get("error", "MAS host failed")
            except (TypeError, ValueError):
                message = "MAS host failed without a valid diagnostic"
            raise HostError(str(message))
        try:
            response = loads_cli_object(completed.stdout)
            if (response.get("status") != "completed"
                    or response.get("run_id") != self.run_id):
                raise HostError("host returned an invalid completion")
            output = response.get("output")
            validate_object(output)
            dumps_object(output)
        except (TypeError, ValueError) as error:
            raise HostError("host returned an invalid completion") from error
        return decode(output, output_type) if output_type is not None else output

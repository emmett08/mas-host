"""Verify reviewed assets and provisioned runtime version labels before import."""
import hashlib
import importlib.metadata
import os
from pathlib import Path, PurePosixPath
import sys

from .contracts import (ADAPTER, BACKEND, CODE_REVISION, MODEL_REVISION, AdapterError,
                        closed, lower_hex, parse, require)

TOKENIZER_URL = "https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken"
TOKENIZER_KEY = hashlib.sha1(TOKENIZER_URL.encode()).hexdigest()
SETTINGS = {"context_tokens": 4096, "max_input_bytes": 16384, "max_findings": 128,
            "max_response_bytes": 65536, "encoding": "o200k_base",
            "decode_mode": "viterbi", "trim_whitespace": False}


def hash_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        while block := stream.read(1048576):
            digest.update(block)
    return digest.hexdigest()


def relative_path(root, name):
    require(isinstance(name, str) and len(name) <= 256, "invalid-manifest")
    path = PurePosixPath(name)
    require(not path.is_absolute() and bool(path.parts)
            and all(part not in {"", ".", ".."} for part in path.parts)
            and str(path) == name and "\\" not in name, "invalid-manifest")
    target = Path(root)
    for part in path.parts:
        target = target / part
        require(not target.is_symlink(), "asset-mismatch")
    require(target.resolve().is_relative_to(Path(root).resolve()), "asset-mismatch")
    return target


def file_record(value):
    closed(value, ("sha256", "bytes"), "invalid-manifest")
    require(lower_hex(value["sha256"], 64) and type(value["bytes"]) is int
            and 0 <= value["bytes"] <= 4 * 1024**3, "invalid-manifest")


def verify_files(root, files):
    require(isinstance(files, dict) and 1 <= len(files) <= 128, "invalid-manifest")
    for name, value in files.items():
        file_record(value)
        target = relative_path(root, name)
        require(target.is_file() and target.stat().st_size == value["bytes"], "asset-mismatch")
        require(hash_file(target) == value["sha256"], "asset-mismatch")


def read_manifest(path):
    with Path(path).open("rb") as stream:
        raw = stream.read(65537)
    value = parse(raw)
    closed(value, ("version", "code_revision", "model_revision", "backend", "adapter",
                   "worker_sha256", "adapter_files", "files", "runtime", "settings"), "invalid-manifest")
    require(value["version"] == "mas-privacy-bundle/1"
            and value["code_revision"] == CODE_REVISION and value["model_revision"] == MODEL_REVISION
            and value["backend"] == BACKEND and value["adapter"] == ADAPTER
            and lower_hex(value["worker_sha256"], 64), "invalid-manifest")
    require(value["settings"] == SETTINGS, "unsupported-settings")
    return value, hashlib.sha256(raw).hexdigest()


def identity(manifest, digest):
    return {"bundle_sha256": digest, **{name: manifest[name]
            for name in ("code_revision", "model_revision", "backend", "adapter")}}


def reviewed_files(adapter_root):
    code = parse((adapter_root / "upstream-code.lock.json").read_bytes())
    assets = parse((adapter_root / "upstream-assets.lock.json").read_bytes())
    require(code["revision"] == CODE_REVISION and assets["code_revision"] == CODE_REVISION
            and assets["model_revision"] == MODEL_REVISION, "invalid-source-lock")
    result = {"source/" + name: record for name, record in code["files"].items()}
    for name, record in assets["assets"].items():
        if name == "tiktoken/o200k_base.tiktoken":
            name = "tokenizer-cache/" + TOKENIZER_KEY
        result[name] = {field: record[field] for field in ("sha256", "bytes") if field in record}
    return result


def verify_assets(manifest, assets_root, adapter_root):
    expected = reviewed_files(adapter_root)
    require(set(manifest["files"]) == set(expected), "asset-mismatch")
    for name, record in expected.items():
        require(all(manifest["files"][name].get(key) == value for key, value in record.items()), "asset-mismatch")
    verify_files(assets_root, manifest["files"])
    source = Path(assets_root) / "source"
    actual_code = {"source/" + str(p.relative_to(source)) for p in source.rglob("*") if p.is_file() or p.is_symlink()}
    require(actual_code == {name for name in expected if name.startswith("source/")}
            and not any(p.is_symlink() for p in source.rglob("*")), "asset-mismatch")
    checkpoint_files = {p.name for p in (Path(assets_root) / "checkpoint").iterdir()}
    require(checkpoint_files == {"config.json", "model.safetensors", "viterbi_calibration.json", "dtypes.json"}, "asset-mismatch")
    require({p.name for p in (Path(assets_root) / "tokenizer-cache").iterdir()} == {TOKENIZER_KEY}, "asset-mismatch")


def verify_runtime(manifest, adapter_root):
    runtime = manifest["runtime"]
    closed(runtime, ("python", "packages"), "invalid-manifest")
    require(runtime["python"] == ".".join(map(str, sys.version_info[:3])), "runtime-mismatch")
    packages = runtime["packages"]
    require(isinstance(packages, dict) and 5 <= len(packages) <= 128
            and {"torch", "tiktoken", "safetensors", "huggingface-hub", "numpy"}.issubset(packages), "runtime-mismatch")
    lock = parse((Path(adapter_root) / "runtime.lock.json").read_bytes())
    pinned = {name.replace("_", "-").lower(): value["version"] for name, value in lock["packages"].items()}
    require(runtime["python"] == lock["python"]["version"] and packages == pinned,
            "runtime-mismatch")
    require(hash_file(sys.executable) == lock["python"]["executable_sha256"], "runtime-mismatch")
    installed = {dist.metadata["Name"].replace("_", "-").lower() for dist in importlib.metadata.distributions()}
    require(installed.issubset(set(pinned) | {"pip", "opf"}), "runtime-mismatch")
    if "opf" in installed:
        require(importlib.metadata.version("opf") == lock["opf"]["version"], "runtime-mismatch")
    for name, version in packages.items():
        require(isinstance(name, str) and isinstance(version, str)
                and 0 < len(name) <= 128 and 0 < len(version) <= 128, "runtime-mismatch")
        try:
            actual = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            raise AdapterError("runtime-mismatch") from None
        require(actual == version, "runtime-mismatch")


def offline_environment(assets_root):
    root = Path(assets_root).resolve()
    os.environ.update({"HF_HUB_OFFLINE": "1", "HF_DATASETS_OFFLINE": "1",
        "TRANSFORMERS_OFFLINE": "1", "HF_HUB_DISABLE_TELEMETRY": "1",
        "TIKTOKEN_CACHE_DIR": str(root / "tokenizer-cache"), "OPF_TORCH_COMPILE": "0",
        "OPF_CHECKPOINT": str(root / "checkpoint"), "PYTHONDONTWRITEBYTECODE": "1",
        "DO_NOT_TRACK": "1", "TOKENIZERS_PARALLELISM": "false"})
    sys.dont_write_bytecode = True


def block_network():
    """Reject Python network use as a fault guard, without claiming OS isolation."""
    def denied(*_args, **_kwargs):
        raise AdapterError("network-disabled")
    import socket
    socket.create_connection = denied
    socket.socket.connect = denied
    socket.socket.connect_ex = denied
    socket.socket.sendto = denied

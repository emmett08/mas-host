"""Provision public pinned inputs separately from offline private-text inference."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import urllib.request

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
from mas_privacy_opf.bundle import (SETTINGS, TOKENIZER_KEY, hash_file, reviewed_files,
                                   verify_assets, verify_runtime)
from mas_privacy_opf.contracts import ADAPTER, BACKEND, CODE_REVISION, MODEL_REVISION, AdapterError


def load(name):
    return json.loads((ROOT / name).read_bytes())


def download(url, path, record):
    if path.exists():
        if path.stat().st_size != record["bytes"] or hash_file(path) != record["sha256"]:
            raise AdapterError("existing-asset-mismatch")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix="." + path.name + ".", suffix=".partial", dir=path.parent)
    temporary = Path(name)
    digest, size = hashlib.sha256(), 0
    try:
        with os.fdopen(descriptor, "wb") as stream, urllib.request.urlopen(url, timeout=60) as response:
            while block := response.read(1048576):
                size += len(block)
                if size > record["bytes"]:
                    raise AdapterError("asset-limit")
                stream.write(block)
                digest.update(block)
        if size != record["bytes"] or digest.hexdigest() != record["sha256"]:
            raise AdapterError("asset-mismatch")
        temporary.rename(path)
    finally:
        temporary.unlink(missing_ok=True)


def fetch(assets):
    source = load("upstream-code.lock.json")
    for name, record in source["files"].items():
        download(f"https://raw.githubusercontent.com/openai/privacy-filter/{CODE_REVISION}/{name}", assets / "source" / name, record)
    for name, record in load("upstream-assets.lock.json")["assets"].items():
        destination = "tokenizer-cache/" + TOKENIZER_KEY if name.startswith("tiktoken/") else name
        download(record["url"], assets / destination, record)


def seal(assets, output):
    runtime = load("runtime.lock.json")
    names = {name.replace("_", "-").lower(): record["version"] for name, record in runtime["packages"].items()}
    files = reviewed_files(ROOT)
    adapter_names = [str(path.relative_to(ROOT)) for path in (ROOT / "mas_privacy_opf").glob("*.py")]
    adapter_names += ["upstream-code.lock.json", "upstream-assets.lock.json", "runtime.lock.json"]
    manifest = {"version": "mas-privacy-bundle/1", "code_revision": CODE_REVISION,
        "model_revision": MODEL_REVISION, "backend": BACKEND, "adapter": ADAPTER,
        "worker_sha256": hash_file(ROOT / "worker.py"),
        "adapter_files": {name: {"sha256": hash_file(ROOT / name), "bytes": (ROOT / name).stat().st_size}
                          for name in sorted(adapter_names)},
        "files": files, "runtime": {"python": runtime["python"]["version"], "packages": names},
        "settings": SETTINGS}
    verify_assets(manifest, assets, ROOT)
    verify_runtime(manifest, ROOT)
    raw = (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode()
    if len(raw) > 65536:
        raise AdapterError("manifest-limit")
    with output.open("xb") as stream:
        stream.write(raw)
    print(json.dumps({"manifest_sha256": hashlib.sha256(raw).hexdigest(), "backend": BACKEND}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    requirements = sub.add_parser("requirements")
    requirements.add_argument("--output", type=Path, required=True)
    fetch_parser = sub.add_parser("fetch")
    fetch_parser.add_argument("--assets", type=Path, required=True)
    seal_parser = sub.add_parser("seal")
    seal_parser.add_argument("--assets", type=Path, required=True)
    seal_parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.action == "requirements":
        lock = load("runtime.lock.json")
        text = "\n".join(f"{name} @ {record['url']} --hash=sha256:{record['hashes']['sha256']}"
                         for name, record in sorted(lock["packages"].items())) + "\n"
        with args.output.open("x") as stream:
            stream.write(text)
    elif args.action == "fetch":
        fetch(args.assets.resolve())
    else:
        seal(args.assets.resolve(), args.output)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("privacy provisioning failed; no inference executed", file=sys.stderr)
        raise SystemExit(1) from None

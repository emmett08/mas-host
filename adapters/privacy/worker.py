"""Serve one private guardian frame after checking the adapter's source closure."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import sys


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    sys.dont_write_bytecode = True
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--assets", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    # No adapter module executes until its deployment-approved hash has matched.
    with Path(args.manifest).open("rb") as stream:
        raw = stream.read(65537)
    if len(raw) > 65536:
        raise ValueError()
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError()
            result[key] = value
        return result
    manifest = json.loads(raw, object_pairs_hook=pairs)
    if hashlib.sha256(Path(__file__).read_bytes()).hexdigest() != manifest["worker_sha256"]:
        raise ValueError()
    files = manifest["adapter_files"]
    modules = {"__init__.py", "contracts.py", "transport.py", "bundle.py", "inference.py", "service.py"}
    actual = {str(path.relative_to(root / "mas_privacy_opf"))
              for path in (root / "mas_privacy_opf").rglob("*") if path.is_file() or path.is_symlink()}
    expected = {"mas_privacy_opf/" + name for name in modules}
    expected.update(("upstream-code.lock.json", "upstream-assets.lock.json", "runtime.lock.json"))
    if set(files) != expected or actual != modules:
        raise ValueError()
    for name, record in files.items():
        path = root / name
        if path.is_symlink() or not path.resolve().is_relative_to(root):
            raise ValueError()
        content = path.read_bytes()
        if len(content) > 1048576 or len(content) != record["bytes"] or hashlib.sha256(content).hexdigest() != record["sha256"]:
            raise ValueError()
    sys.path.insert(0, str(root))
    from mas_privacy_opf.contracts import parse
    from mas_privacy_opf.service import assess
    from mas_privacy_opf.transport import read_frame, write_frame
    fd = int(os.environ["MAS_WORKER_CHANNEL_FD"])
    request = parse(read_frame(fd))
    result = assess(request, args.manifest, args.assets)
    write_frame(fd, json.dumps(result, ensure_ascii=True, separators=(",", ":")).encode())


if __name__ == "__main__":
    try:
        main()
    except BaseException:
        # The guardian observes process failure; no traceback or source is emitted.
        os._exit(1)

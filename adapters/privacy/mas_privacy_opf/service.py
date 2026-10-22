"""Return one safe correlated assessment after offline bundle verification."""
from pathlib import Path

from .bundle import identity, offline_environment, block_network, read_manifest, verify_assets, verify_runtime
from .contracts import AdapterError, OMISSIONS, request, require, response
from .inference import infer

def assess(value, manifest_path, assets_root):
    value = request(value)
    try:
        manifest, digest = read_manifest(manifest_path)
        require(value["detector"] == identity(manifest, digest), "model-unavailable")
        adapter_root = Path(__file__).resolve().parents[1]
        verify_assets(manifest, assets_root, adapter_root)
        verify_runtime(manifest, adapter_root)
        offline_environment(assets_root)
        block_network()
        completion, findings, omissions = infer(value["text"], assets_root, manifest["settings"])
        require(all(code in OMISSIONS for code in omissions), "invalid-model-output")
        return response(value, completion, findings, omissions)
    except AdapterError as error:
        reason = error.code if error.code in OMISSIONS else "model-unavailable"
        return response(value, "unsupported" if reason == "input-token-limit" else "failed", omissions=[reason])
    except BaseException:
        # Underlying libraries can embed input, paths or previews in messages.
        # Only this fixed reason leaves the private worker.
        return response(value, "failed", omissions=["model-inference-failed"])

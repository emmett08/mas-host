"""Run the reviewed OPF classifier with explicit token coverage and decoder guards."""
from dataclasses import replace
from pathlib import Path
import sys

from .contracts import AdapterError, LABELS, MAX_FINDINGS, require


class CheckedModel:
    def __init__(self, model, token_ids, finite):
        self.model, self.token_ids, self.finite = model, list(token_ids), finite
        self.calls = 0

    def __call__(self, tokens, *, attention_mask):
        self.calls += 1
        require(self.calls == 1 and tokens.tolist() == [self.token_ids]
                and attention_mask.tolist() == [[True] * len(self.token_ids)], "incomplete-token-coverage")
        scores = self.model(tokens, attention_mask=attention_mask)
        require(tuple(scores.shape) == (1, len(self.token_ids), 33)
                and self.finite(scores), "incomplete-token-coverage")
        return scores


class CheckedDecoder:
    def __init__(self, decoder, token_count, finite):
        self.decoder, self.token_count, self.finite = decoder, token_count, finite
        self.calls, self.labels = 0, None

    def decode(self, scores):
        self.calls += 1
        require(self.calls == 1 and tuple(scores.shape) == (self.token_count, 33)
                and self.finite(scores), "incomplete-token-coverage")
        labels = self.decoder.decode(scores)
        require(isinstance(labels, (list, tuple)) and len(labels) == self.token_count
                and all(type(label) is int and 0 <= label < 33 for label in labels), "decoder-incomplete")
        self.labels = list(labels)
        return labels


def validate_findings(findings, text):
    result = []
    for start, end, label in findings:
        require(type(start) is int and type(end) is int and 0 <= start < end <= len(text)
                and label in LABELS, "invalid-model-output")
        result.append({"start": start, "end": end, "label": label})
    result.sort(key=lambda item: (item["start"], item["end"], item["label"]))
    require(len({(r["start"], r["end"], r["label"]) for r in result}) == len(result), "invalid-model-output")
    if len(result) > MAX_FINDINGS:
        return "partial", result[:MAX_FINDINGS], ["detector-limit"]
    return "complete", result, []


def infer(text, assets_root, settings):
    # The caller has verified every source file, runtime version, checkpoint and
    # tokenizer cache file before this import. There is no model-ID lookup.
    source = Path(assets_root).resolve() / "source"
    sys.path.insert(0, str(source))
    import opf
    require(Path(opf.__file__).resolve() == source / "opf" / "__init__.py", "model-unavailable")
    from opf import OPF, DecodeOptions
    from opf._core.runtime import predict_text
    from opf._core.spans import decode_text_with_offsets, labels_to_spans
    import torch

    torch.set_num_threads(1)
    torch.set_num_interop_threads(1)
    checkpoint = Path(assets_root).resolve() / "checkpoint"
    engine = OPF(model=str(checkpoint), device="cpu", context_window_length=settings["context_tokens"],
                 trim_whitespace=False, output_mode="typed", decode_mode="viterbi",
                 discard_overlapping_predicted_spans=False, output_text_only=False)
    decode = DecodeOptions(decode_mode="viterbi", viterbi_calibration_path=str(checkpoint / "viterbi_calibration.json"))
    runtime, decoder = engine.get_prediction_components(decode=decode)
    require(runtime.n_ctx == settings["context_tokens"] and str(runtime.device) == "cpu"
            and runtime.active_encoding_name == settings["encoding"] and decoder is not None,
            "model-unavailable")
    require(next(runtime.model.parameters()).dtype == torch.bfloat16, "model-unavailable")
    require(set(runtime.label_info.span_class_names) == LABELS | {"O"}, "invalid-model-output")
    token_ids = tuple(runtime.encoding.encode(text, allowed_special="all"))
    if len(token_ids) > settings["context_tokens"]:
        return "unsupported", [], ["input-token-limit"]
    decoded, starts, ends = decode_text_with_offsets(token_ids, runtime.encoding)
    require(decoded == text and len(starts) == len(ends) == len(token_ids), "source-reconstruction-failed")
    if not token_ids:
        return "complete", [], []

    finite = lambda scores: bool(torch.isfinite(scores).all().item())
    checked_model = CheckedModel(runtime.model, token_ids, finite)
    checked_decoder = CheckedDecoder(decoder, len(token_ids), finite)
    guarded = replace(runtime, model=checked_model)
    with torch.inference_mode():
        prediction = predict_text(guarded, text, decoder=checked_decoder)
    require(checked_model.calls == checked_decoder.calls == 1 and checked_decoder.labels is not None,
            "incomplete-token-coverage")
    require(prediction.text == text and prediction.decoded_mismatch is False, "source-reconstruction-failed")

    # OPF's public display projection removes overlapping character spans. Use
    # the same pinned decoder's complete labels before that lossy display step.
    # Convert every token span explicitly, rejecting rather than dropping one.
    spans = []
    for category, start, end in labels_to_spans(dict(enumerate(checked_decoder.labels)), runtime.label_info):
        require(type(start) is int and type(end) is int and 0 <= start < end <= len(token_ids), "invalid-model-output")
        label = runtime.label_info.span_class_names[category]
        spans.append((starts[start], ends[end - 1], label))
    return validate_findings(spans, text)

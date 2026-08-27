#!/usr/bin/env python3
"""Minimal offline RapidOCR/ONNX adapter for ``mpdf ocr``.

The script intentionally does not install packages, download models, or log
recognized text. Provision ``rapidocr_onnxruntime`` and model files out of
band, then pass this file as ``--provider-executable`` (it is invoked with
argv by the Rust runner).
"""

import argparse
import json
import sys
import unicodedata
from pathlib import Path

MODEL_FILES = (
    "ch_PP-OCRv4_det_infer.onnx",
    "ch_PP-OCRv4_rec_infer.onnx",
    "ch_ppocr_mobile_v2.0_cls_infer.onnx",
)


def run(args: argparse.Namespace) -> int:
    if args.protocol != "mpdf-ocr" or args.protocol_version != "0.1":
        print("unsupported OCR protocol", file=sys.stderr)
        return 78
    model_dir = Path(args.model_dir)
    if any(not (model_dir / name).is_file() for name in MODEL_FILES):
        print("required RapidOCR model file is missing", file=sys.stderr)
        return 78
    request_line = sys.stdin.readline()
    if not request_line:
        print("provider request is missing", file=sys.stderr)
        return 78
    request = json.loads(request_line)
    try:
        from rapidocr_onnxruntime import RapidOCR
    except ImportError:
        print("rapidocr_onnxruntime is not installed", file=sys.stderr)
        return 78
    engine = RapidOCR(
        det_model_path=str(model_dir / MODEL_FILES[0]),
        rec_model_path=str(model_dir / MODEL_FILES[1]),
        cls_model_path=str(model_dir / MODEL_FILES[2]),
    )
    result, _ = engine(args.input)
    blocks = []
    raw_result = []
    for order, item in enumerate(result or []):
        box, text, confidence = item
        text = unicodedata.normalize("NFC", str(text))
        raw_box = [[float(point[0]), float(point[1])] for point in box]
        raw_result.append(
            {"box": raw_box, "text": text, "confidence": float(confidence)}
        )
        xs = [float(point[0]) for point in box]
        ys = [float(point[1]) for point in box]
        bbox = {
            "x": min(xs),
            "y": min(ys),
            "width": max(xs) - min(xs),
            "height": max(ys) - min(ys),
        }
        word = {
            "text": text,
            "normalized_text": " ".join(text.split()),
            "bbox": bbox,
            "confidence": float(confidence),
            "reading_order": order,
        }
        line = {
            "bbox": bbox,
            "confidence": float(confidence),
            "reading_order": order,
            "words": [word],
        }
        blocks.append(
            {
                "bbox": bbox,
                "confidence": float(confidence),
                "reading_order": order,
                "lines": [line],
            }
        )
    from PIL import Image

    with Image.open(args.input) as image:
        width, height = image.size
    response = {
        "protocol": "mpdf-ocr",
        "protocol_version": "0.1",
        "page_index": request["page_index"],
        "input_asset_sha256": request["input_asset_sha256"],
        "width": width,
        "height": height,
        "blocks": blocks,
        "engine": "rapidocr",
        "model": "onnx",
        "version": "provisioned",
        "parameters": {},
        "execution_location": "local",
        "raw_result": raw_result,
    }
    print(json.dumps(response, ensure_ascii=False, separators=(",", ":")))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--protocol", required=True)
    parser.add_argument("--protocol-version", required=True)
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--input", required=True)
    args = parser.parse_args()
    try:
        return run(args)
    except Exception:
        # Never emit a traceback or recognized text to the parent/logs.
        print("local OCR provider failed", file=sys.stderr)
        return 79


if __name__ == "__main__":
    raise SystemExit(main())

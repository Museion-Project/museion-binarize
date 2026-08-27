#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: MPDF_PDFIUM_LIBRARY=<path> $0 <new-output-directory>" >&2
  exit 2
fi
: "${MPDF_PDFIUM_LIBRARY:?MPDF_PDFIUM_LIBRARY must name the tested PDFium library}"

script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$script_dir/../.." && pwd)
output_dir=$1
if [[ -e "$output_dir" ]]; then
  echo "refusing to reuse existing output path: $output_dir" >&2
  exit 2
fi
mkdir -p "$output_dir"

cd "$repo_root"
MPDF_M5_ACCEPTANCE_OUTPUT="$output_dir" cargo test --offline -p mpdf-core \
  --test searchable_pdf m5_pdfium_source_preserving_reopen_and_rotation_fixture \
  -- --nocapture

source_pdf="$output_dir/source-rotations.pdf"
searchable_pdf="$output_dir/searchable-rotations.pdf"

qpdf --check "$source_pdf" >"$output_dir/qpdf-source.txt" 2>&1
qpdf --check "$searchable_pdf" >"$output_dir/qpdf-searchable.txt" 2>&1
qpdf --json --json-key=outlines "$searchable_pdf" >"$output_dir/qpdf-outlines.json"

pdfinfo -f 1 -l 4 "$searchable_pdf" >"$output_dir/poppler-info.txt"
pdftotext -layout "$searchable_pdf" "$output_dir/poppler-text.txt" \
  2>"$output_dir/poppler-stderr.txt"
test ! -s "$output_dir/poppler-stderr.txt"
rg -q 'Ἀρχὴ' "$output_dir/poppler-text.txt"
rg -q 'Πολιτείας' "$output_dir/poppler-text.txt"
rg -q 'Appendix' "$output_dir/poppler-text.txt"

mkdir "$output_dir/ghostscript-source" "$output_dir/ghostscript-searchable"
gs -q -dSAFER -dBATCH -dNOPAUSE -sDEVICE=png16m -r72 \
  -sOutputFile="$output_dir/ghostscript-source/page-%02d.png" "$source_pdf"
gs -q -dSAFER -dBATCH -dNOPAUSE -sDEVICE=png16m -r72 \
  -sOutputFile="$output_dir/ghostscript-searchable/page-%02d.png" "$searchable_pdf"
for source_page in "$output_dir"/ghostscript-source/*.png; do
  page_name=$(basename "$source_page")
  cmp "$source_page" "$output_dir/ghostscript-searchable/$page_name"
done

swift "$script_dir/pdfkit_check.swift" "$searchable_pdf" \
  >"$output_dir/pdfkit.json"
shasum -a 256 "$source_pdf" "$searchable_pdf" "$output_dir/ground-truth.json" \
  >"$output_dir/sha256.txt"

echo "M5 automated reader matrix passed: $output_dir"

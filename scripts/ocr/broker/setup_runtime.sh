#!/bin/sh
set -eu
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
python -m pip install --no-deps --target "$repo_dir/.runtime/surya-compat" 'transformers==4.56.2' 'huggingface-hub==0.35.3' 'tokenizers==0.22.2'

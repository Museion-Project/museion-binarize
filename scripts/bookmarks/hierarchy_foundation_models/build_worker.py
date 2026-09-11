"""Build against the actual SDK; never claim an image branch compiled on SDK26."""
import argparse
import json
from pathlib import Path
import subprocess


def build(output):
    output = Path(output).resolve()
    if output.exists():
        raise FileExistsError(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    sdk = subprocess.check_output(['xcrun', '--sdk', 'macosx', '--show-sdk-version'], text=True).strip()
    image = int(sdk.split('.')[0]) >= 27
    source = Path(__file__).with_name('model_worker.swift')
    command = ['xcrun', 'swiftc', '-parse-as-library', '-target', 'arm64-apple-macos26.0',
               '-module-cache-path', str(output.parent/'module-cache')]
    if image:
        command += ['-D', 'FM_HAS_MACOS_27_SDK']
    command += [str(source), '-o', str(output)]
    subprocess.run(command, check=True)
    receipt = dict(sdk=sdk, image_input_compiled=image, command=command,
                   scope=('Image-input branch compiled; actual model inference still requires runtime validation' if image else
                          'SDK26 explicit-unavailable guard only; image inference branch requires a successful SDK27+ build'))
    output.with_suffix('.build.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(json.dumps(receipt))


if __name__ == '__main__':
    p = argparse.ArgumentParser(); p.add_argument('output'); build(p.parse_args().output)

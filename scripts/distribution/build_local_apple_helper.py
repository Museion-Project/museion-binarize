"""Build the known Apple helper source once, recording actual compiler/SDK inputs.
No recognition, signing command, download, old binary relabelling, or replacement.
"""
import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path
from .package_local_candidate import sha, output_guard


def build(source, expected_sha256, output):
    source=Path(source).resolve(strict=True);output=output_guard(output)
    if sha(source)!=expected_sha256:raise ValueError('HELPER_SOURCE_CHANGED')
    def read(args):return subprocess.check_output(args,text=True).strip()
    # Preserve argv[0]: swiftc is a driver symlink whose basename selects driver
    # behavior. Executing its resolved swift-frontend target changes semantics.
    compiler=Path(read(['/usr/bin/xcrun','--find','swiftc'])).absolute()
    sdk=Path(read(['/usr/bin/xcrun','--sdk','macosx','--show-sdk-path'])).resolve()
    compiler_version=read([str(compiler),'--version'])
    sdk_version=read(['/usr/bin/xcrun','--sdk','macosx','--show-sdk-version'])
    output.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='local-helper-cache-') as cache:
        command=[str(compiler),'-O','-target','arm64-apple-macos27.0','-sdk',str(sdk),
                 '-module-cache-path',cache,str(source),'-o',str(output/'apple-accurate')]
        environment=dict(os.environ,CLANG_MODULE_CACHE_PATH=cache)
        result=subprocess.run(command,capture_output=True,text=True,env=environment,timeout=120)
        record=dict(schema='apple-helper-build/1',source_path=str(source),source_sha256=expected_sha256,
                    command=command,compiler_sha256=sha(compiler),compiler_version=compiler_version,
                    sdk=str(sdk),sdk_version=sdk_version,target='arm64-apple-macos27.0',
                    returncode=result.returncode,stdout=result.stdout,stderr=result.stderr,
                    binary_sha256=sha(output/'apple-accurate') if (output/'apple-accurate').exists() else None,
                    source_unchanged=sha(source)==expected_sha256,recognizer_calls=0)
        (output/'build-receipt.json').write_text(json.dumps(record,indent=2))
        if result.returncode or not record['source_unchanged']:raise ValueError('HELPER_BUILD_FAILED')
    return record


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--source',type=Path,required=True)
    p.add_argument('--source-sha256',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    record=build(a.source,a.source_sha256,a.output)
    print(json.dumps({k:record[k] for k in ('schema','target','binary_sha256','returncode','recognizer_calls')}))


if __name__=='__main__':main()

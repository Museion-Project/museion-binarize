#!/usr/bin/env python3
"""Codesigns a built macOS `.app` bundle as a whole.

Why this exists: Rust's linker ad-hoc-signs each individual Apple
Silicon Mach-O binary at build time (arm64 requires every binary to
carry *some* signature), but `tauri-bundler` does not resign the
*bundle* itself when no `signingIdentity` is configured in
`tauri.conf.json` (this project's case — see docs/releasing.md,
"Signing and notarization"). The result is a bundle whose main
executable's embedded CodeDirectory claims a signed structure with a
resource envelope, but `Contents/_CodeSignature/CodeResources` — which
should seal `Contents/Resources` (the bundled PDFium library, the app
icon) — was never generated.

`codesign --verify --deep --strict` on that bundle fails with "code has
no resources but signature indicates they must be present," and macOS
Gatekeeper reports this to the end user as "*is damaged and can't be
opened*," not the expected "*from an unidentified developer*" prompt a
plain, properly ad-hoc-signed unsigned app would get. See
docs/desktop-testing.md, "Milestone 7A," for how this was found and
reproduced.

Signing the whole bundle (even ad-hoc, identity `-`) generates the
missing `CodeResources` seal and fixes this. Ad-hoc signing does not
satisfy `spctl -a` (Gatekeeper's full assessment) or notarization —
those still require real Apple Developer ID credentials, which this
script does not have or claim to provide.

Usage:
    python3 scripts/distribution/sign_macos_app.py \\
        --app-path "target/aarch64-apple-darwin/release/bundle/macos/M PDF Processor.app" \\
        [--identity "Developer ID Application: ..."]
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


def sign(app_path: Path, identity: str) -> None:
    if not app_path.is_dir() or app_path.suffix != ".app":
        raise SystemExit(f"not an .app bundle: {app_path}")
    subprocess.run(
        ["codesign", "--force", "--deep", "--sign", identity, str(app_path)],
        check=True,
    )
    subprocess.run(
        ["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app_path)],
        check=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app-path", required=True, type=Path)
    parser.add_argument(
        "--identity",
        default="-",
        help='codesign identity to sign with; "-" (default) for an ad-hoc signature',
    )
    args = parser.parse_args()
    sign(args.app_path, args.identity)
    print(f"signed and verified: {args.app_path} (identity={args.identity})")
    return 0


if __name__ == "__main__":
    sys.exit(main())

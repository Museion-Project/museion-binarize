#!/usr/bin/env python3
"""Builds (and, with real credentials, signs) the Mac App Store variant
of the desktop app — separate from `package_macos_dmg.py`'s GitHub
Developer-ID/ad-hoc path. See docs/mac-app-store-readiness.md.

Two modes:

- **Structural build (default, no credentials needed)**: stages PDFium,
  renders the entitlements plist (requires `APPLE_TEAM_ID` — not
  secret, but owner-specific), and runs `tauri build --config
  tauri.mas.conf.json --bundles app`. Without `APPLE_SIGNING_IDENTITY`
  set, Tauri's own bundler does not sign the bundle with a real
  identity, so the result is unsigned/ad-hoc-linker-signed only — this
  script does **not** apply GitHub's ad-hoc structural-signing fallback
  to it (see `sign_macos_app.py`'s docstring for that separate,
  intentionally-different path). Structural checks (bundle
  identifier/version consistency, bundled PDFium present and matching
  architecture, entitlements template contains exactly the intended
  keys) always run.

- **Signed (`--sign`, requires `APPLE_SIGNING_IDENTITY`)**: an "Apple
  Distribution" certificate identity already imported into the local
  keychain. Tauri's bundler picks up `APPLE_SIGNING_IDENTITY` from the
  environment and signs the `.app` with the rendered entitlements as
  part of its own build/bundle step — not a separate re-signing pass
  after the fact, which is what let a stale signature survive
  undetected in the GitHub distribution path before the M7A fix (see
  `docs/desktop-testing.md`, "Milestone 7A"). After a signed build,
  `codesign --verify --deep --strict` and an embedded-entitlements
  inspection both run, asserting the App Sandbox entitlement is present
  and no forbidden broad entitlement made it into the final signature.

- **`--package-pkg` (requires `APPLE_INSTALLER_SIGNING_IDENTITY`,
  implies `--sign`)**: signs a `.pkg` installer with a separate "Mac
  Installer Distribution" certificate identity via `productbuild`, the
  artifact format Apple actually accepts at App Store Connect upload —
  this script never uploads it.

Usage:
    # Structural build only, no credentials:
    python3 scripts/distribution/package_mas.py \\
        --target-triple aarch64-apple-darwin --version 0.1.0 \\
        --out-dir target/aarch64-apple-darwin/release/bundle/mas

    # Real signed build (owner credentials required):
    APPLE_TEAM_ID=... APPLE_SIGNING_IDENTITY="Apple Distribution: ..." \\
      python3 scripts/distribution/package_mas.py --sign \\
        --target-triple aarch64-apple-darwin --version 0.1.0 \\
        --out-dir target/aarch64-apple-darwin/release/bundle/mas

    # Signed .pkg ready for (manual, separate) App Store Connect upload:
    APPLE_TEAM_ID=... APPLE_SIGNING_IDENTITY="Apple Distribution: ..." \\
      APPLE_INSTALLER_SIGNING_IDENTITY="Mac Installer Distribution: ..." \\
      python3 scripts/distribution/package_mas.py --package-pkg \\
        --target-triple aarch64-apple-darwin --version 0.1.0 \\
        --out-dir target/aarch64-apple-darwin/release/bundle/mas
"""

from __future__ import annotations

import argparse
import json
import os
import plistlib
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import naming  # noqa: E402
import render_mas_entitlements  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
SRC_TAURI = REPO_ROOT / "apps" / "desktop" / "src-tauri"
DESKTOP_DIR = REPO_ROOT / "apps" / "desktop"
MAS_CONFIG_PATH = SRC_TAURI / "tauri.mas.conf.json"

REQUIRED_ENTITLEMENTS = {
    "com.apple.security.app-sandbox",
    "com.apple.security.files.user-selected.read-write",
    # Required by WKWebView's out-of-process architecture, not by any
    # network code in this app — without it the WebContent XPC service
    # never starts under App Sandbox and the window renders blank. Kept
    # in REQUIRED (not merely "allowed") precisely because dropping it
    # silently produces an app that launches but shows nothing, which is
    # exactly how it was found. See docs/mac-app-store-readiness.md.
    "com.apple.security.network.client",
}

# Broad entitlements this application has no demonstrated need for (see
# the M7B1 preflight audit in docs/mac-app-store-readiness.md). Any of
# these appearing in the entitlements plist — template, rendered, or
# actually embedded in a signed binary — is a structural regression,
# not a judgment call to re-litigate per build.
#
# Note `network.server` (accepting *incoming* connections) stays
# forbidden; only `network.client` is permitted, and only for the
# WebKit reason documented above. The prefix is deliberately not the
# broader "com.apple.security.network." so that a future
# `network.server` addition still fails closed.
FORBIDDEN_ENTITLEMENT_PREFIXES = (
    "com.apple.security.network.server",
    "com.apple.security.automation.",
    "com.apple.security.device.camera",
    "com.apple.security.device.microphone",
    "com.apple.security.personal-information.",
    "com.apple.security.files.downloads.",
    "com.apple.security.files.all",
    "com.apple.security.temporary-exception.",
)


def validate_entitlements_dict(entitlements: dict, *, source: str) -> None:
    missing = REQUIRED_ENTITLEMENTS - entitlements.keys()
    if missing:
        raise SystemExit(f"{source}: missing required entitlement(s): {sorted(missing)}")
    if entitlements.get("com.apple.security.app-sandbox") is not True:
        raise SystemExit(f"{source}: com.apple.security.app-sandbox is not `true`")
    forbidden_found = [
        key
        for key in entitlements
        if any(key.startswith(prefix) for prefix in FORBIDDEN_ENTITLEMENT_PREFIXES)
    ]
    if forbidden_found:
        raise SystemExit(f"{source}: forbidden broad entitlement(s) present: {sorted(forbidden_found)}")


def validate_entitlements_plist_file(plist_path: Path) -> None:
    with plist_path.open("rb") as f:
        entitlements = plistlib.load(f)
    validate_entitlements_dict(entitlements, source=str(plist_path))


def bundle_identifier_and_version() -> tuple[str, str]:
    config = json.loads((SRC_TAURI / "tauri.conf.json").read_text())
    return config["identifier"], config["version"]


def find_built_app(target_triple: str) -> Path:
    macos_dir = REPO_ROOT / "target" / target_triple / "release" / "bundle" / "macos"
    apps = sorted(macos_dir.glob("*.app")) if macos_dir.is_dir() else []
    if len(apps) != 1:
        raise SystemExit(f"expected exactly 1 .app in {macos_dir}, found {len(apps)}: {apps}")
    return apps[0]


def check_bundled_pdfium_matches_arch(app_path: Path) -> None:
    lib = app_path / "Contents" / "Resources" / "libpdfium.dylib"
    if not lib.is_file():
        raise SystemExit(f"bundled PDFium library missing at {lib}")
    exe_candidates = list((app_path / "Contents" / "MacOS").glob("*"))
    if not exe_candidates:
        raise SystemExit(f"no executable found under {app_path}/Contents/MacOS")

    def arch_of(path: Path) -> str:
        out = subprocess.run(["file", str(path)], capture_output=True, text=True, check=True).stdout
        if "arm64" in out:
            return "arm64"
        if "x86_64" in out:
            return "x86_64"
        raise SystemExit(f"could not determine architecture of {path}: {out}")

    app_arch = arch_of(exe_candidates[0])
    lib_arch = arch_of(lib)
    if app_arch != lib_arch:
        raise SystemExit(f"architecture mismatch: app={app_arch} pdfium={lib_arch}")
    print(f"bundled PDFium architecture OK: {app_arch}")


def check_no_dev_absolute_paths(*paths: Path) -> None:
    home = str(Path.home())
    for path in paths:
        if not path.is_file():
            continue
        text = path.read_text(errors="ignore")
        if home in text:
            raise SystemExit(f"{path} contains a developer-machine absolute path ({home})")


def run_tauri_build(target_triple: str, *, signing_identity: str | None) -> None:
    env = os.environ.copy()
    if signing_identity:
        env["APPLE_SIGNING_IDENTITY"] = signing_identity
    subprocess.run(
        [
            "pnpm", "tauri", "build",
            "--target", target_triple,
            "--config", "src-tauri/tauri.mas.conf.json",
            "--bundles", "app",
            # Selects OutputWriteStrategy::DirectWriteToDestination in
            # worker.rs — see docs/mac-app-store-readiness.md, "Sandboxed
            # output-save architecture." Never set for the GitHub build.
            "--features", "mas-sandbox",
        ],
        cwd=DESKTOP_DIR,
        env=env,
        check=True,
    )


def package_pkg(app_path: Path, installer_identity: str, out_path: Path) -> None:
    subprocess.run(
        [
            "xcrun", "productbuild",
            "--sign", installer_identity,
            "--component", str(app_path), "/Applications",
            str(out_path),
        ],
        check=True,
    )
    subprocess.run(["pkgutil", "--check-signature", str(out_path)], check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--target-triple", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--sign", action="store_true", help="require a real Apple Distribution identity")
    parser.add_argument("--package-pkg", action="store_true", help="also build a signed .pkg (implies --sign)")
    args = parser.parse_args()

    sign_required = args.sign or args.package_pkg
    signing_identity = os.environ.get("APPLE_SIGNING_IDENTITY")
    if sign_required and not signing_identity:
        raise SystemExit(
            "error: --sign/--package-pkg requires APPLE_SIGNING_IDENTITY (a real "
            "\"Apple Distribution\" certificate identity already in the local "
            "keychain) — this script never falls back to ad-hoc signing for a "
            "Mac App Store artifact. See docs/mac-app-store-readiness.md."
        )

    # Structural validation of the entitlements *template* runs
    # unconditionally and needs no credentials — it is what guards
    # against a forbidden entitlement being added at all, independent
    # of whether anyone has signed anything yet.
    validate_entitlements_plist_file(SRC_TAURI / "entitlements.mas.plist.template")

    render_mas_entitlements.main()  # requires APPLE_TEAM_ID; fails closed if unset
    validate_entitlements_plist_file(SRC_TAURI / "entitlements.mas.plist")

    subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts" / "distribution" / "stage_desktop_pdfium.py"), args.target_triple],
        check=True,
    )

    run_tauri_build(args.target_triple, signing_identity=signing_identity if sign_required else None)

    app_path = find_built_app(args.target_triple)
    identifier, version = bundle_identifier_and_version()
    if version != args.version:
        raise SystemExit(f"version mismatch: tauri.conf.json has {version!r}, --version was {args.version!r}")
    print(f"bundle identifier/version OK: {identifier} {version}")

    check_bundled_pdfium_matches_arch(app_path)
    check_no_dev_absolute_paths(MAS_CONFIG_PATH, SRC_TAURI / "entitlements.mas.plist")

    if sign_required:
        subprocess.run(
            ["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app_path)],
            check=True,
        )
        embedded = subprocess.run(
            ["codesign", "-d", "--entitlements", ":-", "--xml", str(app_path)],
            capture_output=True, check=True,
        ).stdout
        validate_entitlements_dict(plistlib.loads(embedded), source=f"{app_path} (embedded signature)")
        print(f"signed MAS app verified: {app_path} (identity={signing_identity})")
    else:
        print(
            f"structural build only (unsigned): {app_path}\n"
            "Mac App Store signing pending owner credentials — re-run with --sign "
            "and APPLE_SIGNING_IDENTITY set to actually sign."
        )

    if args.package_pkg:
        installer_identity = os.environ.get("APPLE_INSTALLER_SIGNING_IDENTITY")
        if not installer_identity:
            raise SystemExit(
                "error: --package-pkg requires APPLE_INSTALLER_SIGNING_IDENTITY (a "
                "real \"Mac Installer Distribution\" certificate identity)."
            )
        args.out_dir.mkdir(parents=True, exist_ok=True)
        pkg_path = args.out_dir / naming.desktop_artifact_name(args.version, args.target_triple, "pkg")
        package_pkg(app_path, installer_identity, pkg_path)
        print(f"built and verified signed .pkg: {pkg_path}")

    return 0


if __name__ == "__main__":
    sys.exit(main())

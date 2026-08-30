#!/usr/bin/env python3
"""Fail-closed macOS signing and notarization phases.

``sign-app`` signs and verifies the final app, then writes a marker.  The
separate ``notarize-dmg`` phase only submits the already-created DMG and never
re-signs the app. With no Apple credentials, ``sign-app`` retains the tested
ad-hoc structural path; any partial credential set is an error.
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Callable, Mapping

Runner = Callable[..., subprocess.CompletedProcess]
SIGNING_SECRETS = ("APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_SIGNING_IDENTITY")
NOTARY_SECRETS = ("APPLE_API_KEY", "APPLE_API_KEY_ID", "APPLE_API_ISSUER")
ALL_SECRETS = SIGNING_SECRETS + NOTARY_SECRETS
MACHO_MAGICS = {
    b"\xce\xfa\xed\xfe",  # 32-bit little-endian
    b"\xcf\xfa\xed\xfe",  # 64-bit little-endian
    b"\xfe\xed\xfa\xce",  # 32-bit big-endian
    b"\xfe\xed\xfa\xcf",  # 64-bit big-endian
    b"\xca\xfe\xba\xbe",  # universal
    b"\xbe\xba\xfe\xca",  # universal, byte-swapped
    b"\xca\xfe\xba\xbf",  # universal 64-bit
    b"\xbf\xba\xfe\xca",  # universal 64-bit, byte-swapped
}


def credential_mode(env: Mapping[str, str] | None = None) -> str:
    source = os.environ if env is None else env
    values = {name: bool(source.get(name, "")) for name in ALL_SECRETS}
    present = [name for name, value in values.items() if value]
    if not present:
        return "ad_hoc"
    if len(present) != len(ALL_SECRETS):
        missing = [name for name, value in values.items() if not value]
        raise ValueError("partial Apple signing/notarization credentials; missing: " + ", ".join(missing))
    return "developer_id"


def _run(command: list[str], *, runner: Runner = subprocess.run, **kwargs):
    try:
        return runner(command, check=True, **kwargs)
    except subprocess.CalledProcessError as exc:
        # The security-tool commands contain certificate/keychain passwords.
        # Never let CalledProcessError's default repr echo the full argv in a
        # CI traceback; retain only the executable and exit status.
        raise RuntimeError(f"{command[0]} failed (exit status {exc.returncode})") from None


def _write_secret(value: str, path: Path) -> None:
    try:
        decoded = base64.b64decode(value, validate=True)
        if decoded and decoded != value.encode():
            path.write_bytes(decoded)
            path.chmod(0o600)
            return
    except Exception:
        pass
    path.write_text(value)
    path.chmod(0o600)


def _nested_macho_files(app_path: Path) -> list[Path]:
    """Return bare Mach-O payloads that the outer bundle signature misses.

    Tauri's PDFium sidecar lives in ``Contents/Resources``.  Apple's
    ``codesign --deep`` does not reliably give bare Mach-O files in resource
    directories their own Developer ID signature, and notarization rejects
    them even when the outer app verifies locally.  Sign these payloads
    explicitly, deepest first, before sealing the app bundle.
    """
    contents = app_path / "Contents"
    macos = contents / "MacOS"
    nested: list[Path] = []
    if not contents.is_dir():
        return nested
    for candidate in contents.rglob("*"):
        if not candidate.is_file() or candidate.is_symlink():
            continue
        try:
            candidate.relative_to(macos)
            continue
        except ValueError:
            pass
        try:
            if candidate.read_bytes()[:4] in MACHO_MAGICS:
                nested.append(candidate)
        except OSError:
            continue
    return sorted(nested, key=lambda path: (-len(path.parts), str(path)))


def _codesign_command(path: Path, identity: str, *, developer_id: bool,
                      deep: bool) -> list[str]:
    command = ["codesign", "--force"]
    if deep:
        command.append("--deep")
    if developer_id:
        command += ["--options", "runtime", "--timestamp"]
    command += ["--sign", identity, str(path)]
    return command


def sign_and_verify(app_path: Path, identity: str, *, developer_id: bool,
                    runner: Runner = subprocess.run) -> None:
    for nested in _nested_macho_files(app_path):
        _run(_codesign_command(nested, identity, developer_id=developer_id, deep=False),
             runner=runner)
    _run(_codesign_command(app_path, identity, developer_id=developer_id, deep=True),
         runner=runner)
    _run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app_path)], runner=runner)


def _parse_keychains(output: str | bytes) -> list[str]:
    text = output.decode() if isinstance(output, bytes) else output
    quoted = re.findall(r'"([^"]+)"', text)
    if quoted:
        return quoted
    return [token for token in shlex.split(text) if token not in {"list-keychains", "-d", "user"}]


def _read_keychains(*, runner: Runner) -> list[str]:
    result = _run(["security", "list-keychains", "-d", "user"], runner=runner,
                  capture_output=True, text=True)
    return _parse_keychains(getattr(result, "stdout", ""))


def _restore_keychains(original: list[str], *, runner: Runner) -> None:
    if not original:
        # `security list-keychains -s` with no arguments is not a no-op: it
        # would replace the user's search list with an empty list. Deleting
        # the temporary keychain is the safe cleanup for this case.
        return
    runner(["security", "list-keychains", "-d", "user", "-s", *original], check=True,
           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def notarize(dmg_path: Path, key_path: Path, key_id: str, issuer: str,
             *, runner: Runner = subprocess.run) -> None:
    result = _run(["xcrun", "notarytool", "submit", str(dmg_path), "--key", str(key_path),
                   "--key-id", key_id, "--issuer", issuer, "--wait", "--output-format", "json"],
                  runner=runner, capture_output=True, text=True)
    try:
        payload = json.loads(getattr(result, "stdout", ""))
    except (TypeError, json.JSONDecodeError) as exc:
        raise RuntimeError("notarytool returned invalid JSON") from exc
    if not isinstance(payload, dict) or payload.get("status") != "Accepted":
        raise RuntimeError(f"notarytool did not accept the DMG (status={payload.get('status')!r})")
    _run(["xcrun", "stapler", "staple", str(dmg_path)], runner=runner)
    _run(["xcrun", "stapler", "validate", str(dmg_path)], runner=runner)
    _run(["spctl", "-a", "-vv", "--type", "open", str(dmg_path)], runner=runner)


def sign_app(app_path: Path, *, marker: Path, env: Mapping[str, str] | None = None,
             runner: Runner = subprocess.run) -> dict[str, str]:
    environment = os.environ if env is None else env
    mode = credential_mode(environment)
    marker.parent.mkdir(parents=True, exist_ok=True)
    result = {"signing_state": "ad_hoc", "notarization_state": "pending_credentials"}
    keychain: Path | None = None
    temp_root: Path | None = None
    original_keychains: list[str] = []
    keychains_change_attempted = False
    restore_error: Exception | None = None
    try:
        if mode == "ad_hoc":
            sign_and_verify(app_path, "-", developer_id=False, runner=runner)
        else:
            temp_root = Path(tempfile.mkdtemp(prefix="mpdf-sign-"))
            keychain = temp_root / "build.keychain-db"
            cert_file = temp_root / "certificate.p12"
            _write_secret(environment["APPLE_CERTIFICATE"], cert_file)
            password = environment["APPLE_CERTIFICATE_PASSWORD"]
            original_keychains = _read_keychains(runner=runner)
            _run(["security", "create-keychain", "-p", password, str(keychain)], runner=runner)
            _run(["security", "set-keychain-settings", "-lut", "21600", str(keychain)], runner=runner)
            _run(["security", "unlock-keychain", "-p", password, str(keychain)], runner=runner)
            _run(["security", "import", str(cert_file), "-k", str(keychain), "-P", password,
                  "-T", "/usr/bin/codesign"], runner=runner)
            _run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s",
                  "-k", password, str(keychain)], runner=runner)
            # Mark this before invoking security: if the command partially
            # succeeds and then reports an error, the finally block must still
            # restore the saved user search list.
            keychains_change_attempted = True
            _run(["security", "list-keychains", "-d", "user", "-s", *original_keychains, str(keychain)], runner=runner)
            sign_and_verify(app_path, environment["APPLE_SIGNING_IDENTITY"], developer_id=True, runner=runner)
            result["signing_state"] = "signed"
        marker.write_text(json.dumps(result, sort_keys=True) + "\n")
        return result
    finally:
        if keychain is not None:
            try:
                if keychains_change_attempted:
                    _restore_keychains(original_keychains, runner=runner)
            except Exception as exc:
                restore_error = exc
            try:
                runner(["security", "delete-keychain", str(keychain)], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            except Exception:
                pass
        if temp_root is not None:
            shutil.rmtree(temp_root, ignore_errors=True)
        if restore_error is not None:
            raise RuntimeError("failed to restore the user's keychain search list") from restore_error


def notarize_dmg(dmg_path: Path, *, marker: Path, env: Mapping[str, str] | None = None,
                 runner: Runner = subprocess.run) -> dict[str, str]:
    environment = os.environ if env is None else env
    if credential_mode(environment) != "developer_id":
        raise ValueError("notarization requires the complete Apple credential set")
    if not marker.is_file():
        raise ValueError("cannot notarize without a signing marker")
    try:
        state = json.loads(marker.read_text())
    except (OSError, json.JSONDecodeError, TypeError) as exc:
        raise ValueError("invalid signing marker") from exc
    if not isinstance(state, dict):
        raise ValueError("invalid signing marker")
    if state.get("signing_state") != "signed" or state.get("notarization_state") != "pending_credentials":
        raise ValueError("notarization requires marker state signed/pending_credentials")
    temp_root = Path(tempfile.mkdtemp(prefix="mpdf-notary-"))
    key_file = temp_root / "notary-key.p8"
    try:
        _write_secret(environment["APPLE_API_KEY"], key_file)
        notarize(dmg_path, key_file, environment["APPLE_API_KEY_ID"], environment["APPLE_API_ISSUER"], runner=runner)
        state["notarization_state"] = "notarized"
        marker.write_text(json.dumps(state, sort_keys=True) + "\n")
        return state
    finally:
        shutil.rmtree(temp_root, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("sign-app", "notarize-dmg"), default="sign-app")
    parser.add_argument("--app-path", type=Path)
    parser.add_argument("--dmg-path", type=Path)
    parser.add_argument("--marker", required=True, type=Path)
    args = parser.parse_args()
    if args.phase == "sign-app":
        if not args.app_path or not args.app_path.is_dir() or args.app_path.suffix != ".app":
            raise SystemExit(f"not an .app bundle: {args.app_path}")
        result = sign_app(args.app_path, marker=args.marker)
    else:
        if not args.dmg_path or not args.dmg_path.is_file():
            raise SystemExit(f"not a DMG file: {args.dmg_path}")
        result = notarize_dmg(args.dmg_path, marker=args.marker)
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

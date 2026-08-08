#!/usr/bin/env python3
"""Renders `entitlements.mas.plist` from `entitlements.mas.plist.template`.

The template is committed (no secrets in it — a Team ID is not secret,
but is still owner-specific and environment-dependent, so it does not
belong hard-coded in a file every contributor's checkout shares); the
rendered output is not (see `.gitignore`). The bundle identifier is read
from `tauri.conf.json` itself, not passed separately, so it can never
drift from the identifier the rest of the build actually uses.

Usage:
    APPLE_TEAM_ID=ABCDE12345 python3 scripts/distribution/render_mas_entitlements.py
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SRC_TAURI = REPO_ROOT / "apps" / "desktop" / "src-tauri"
TEMPLATE_PATH = SRC_TAURI / "entitlements.mas.plist.template"
BASE_CONFIG_PATH = SRC_TAURI / "tauri.conf.json"
TEAM_ID_ENV = "APPLE_TEAM_ID"


def bundle_identifier() -> str:
    config = json.loads(BASE_CONFIG_PATH.read_text())
    identifier = config.get("identifier")
    if not identifier:
        raise SystemExit(f"{BASE_CONFIG_PATH} has no top-level \"identifier\"")
    return identifier


def render(team_id: str, identifier: str) -> str:
    template = TEMPLATE_PATH.read_text()
    return template.replace("@TEAM_ID@", team_id).replace("@BUNDLE_IDENTIFIER@", identifier)


def main() -> int:
    team_id = os.environ.get(TEAM_ID_ENV)
    if not team_id:
        raise SystemExit(
            f"error: {TEAM_ID_ENV} is not set. A Mac App Store entitlements file "
            "requires the owner's Apple Developer Team ID — see "
            "docs/mac-app-store-readiness.md, \"Owner action required.\""
        )

    identifier = bundle_identifier()
    rendered = render(team_id, identifier)
    out_path = SRC_TAURI / "entitlements.mas.plist"
    out_path.write_text(rendered)
    print(f"rendered {out_path} (team_id={team_id}, identifier={identifier})")
    return 0


if __name__ == "__main__":
    sys.exit(main())

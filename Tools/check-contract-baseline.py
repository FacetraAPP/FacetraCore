#!/usr/bin/env python3
"""Require explicit compatibility review when the reviewed v1 schema changes."""
import hashlib
import json
from pathlib import Path


def fingerprint(value):
    # Ignore formatting and object-key order, including generator refactors.
    canonical = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def main():
    directory = Path(__file__).resolve().parent.parent / "Contracts/charting/v1"
    baseline = json.loads((directory / "baseline.json").read_text())
    for kind in ("chart", "dashboard", "common"):
        name = f"{kind}.schema.json"
        actual = fingerprint(json.loads((directory / name).read_text()))
        if actual != baseline["schemaSha256"][name]:
            raise SystemExit(
                f"Reviewed v1 schema changed: {name}. See Contracts/charting/v1/specification.md. "
                "Regenerating schemas does not approve a contract change; introduce a new "
                "major after release, or document and review a pre-release v1 extension."
            )
    print("Charting v1: reviewed schema baseline passed")


if __name__ == "__main__":
    main()

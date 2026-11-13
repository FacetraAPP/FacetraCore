#!/usr/bin/env python3
"""Keep facetra-contract free of execution, database and platform crates."""
import json
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
cargo = ["rustup", "run", "1.90.0", "cargo"] if shutil.which("rustup") else ["cargo"]
metadata = json.loads(
    subprocess.check_output(
        cargo
        + [
            "metadata",
            "--locked",
            "--format-version=1",
            "--manifest-path",
            str(root / "contract" / "Cargo.toml"),
        ]
    )
)
packages = {p["id"]: p["name"] for p in metadata["packages"]}
graph = {n["id"]: n["dependencies"] for n in metadata["resolve"]["nodes"]}
seen = set()
pending = [metadata["resolve"]["root"]]
while pending:
    node = pending.pop()
    if node not in seen:
        seen.add(node)
        pending.extend(graph[node])

banned = {
    "duckdb",
    "libduckdb-sys",
    "reqwest",
    "hyper",
    "tokio",
    "tokio-postgres",
    "mysql_async",
    "rusqlite",
    "native-tls",
    "security-framework",
    "facetra-pipeline",
}
found = sorted({packages[node] for node in seen} & banned)
assert not found, ("facetra-contract", found)
print(f"facetra-contract: dependency boundary passed ({len(seen)} packages)")

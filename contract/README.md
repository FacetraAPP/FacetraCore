# facetra-contract

Platform-independent Chart/Dashboard YAML v1 models, JSON Schema generation,
parameter handling, static workspace checks and reference resolution.
[Workspace identity text](../Contracts/workspace/v1/specification.md) is validated
separately as fixed strings; workspace translation fields are rejected.

v1 is stable. Its typed format and behavior are frozen; the schema baseline and
behavioral regressions are checked independently of schema generation. See the
[stability review](../Contracts/charting/v1/stability-review.md) before proposing
a format change.

See [the specification](../Contracts/charting/v1/specification.md) for parsing
semantics, validation levels, diagnostics and compatibility. Run independently:

```sh
cd contract
cargo +1.90.0 test --locked
```

Regenerate assets with `Tools/check-contract.sh --write` from the repository root;
run `Tools/check-contract.sh` to verify checked-in assets and samples.

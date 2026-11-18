# ADR-0002: The Public API Is Not Frozen

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

`mutil-engine` and the `demo` harness have no external consumers yet. Agents have still treated the public surface (types, fields, functions, variants, rule IDs, the demo's C ABI and JSON) as fixed. They have kept awkward signatures, added shims, or skipped a simplification only to keep it unchanged. This keeps poor designs around and adds code.

## Decision

1. **Public APIs may change at any time.** Renaming, removing, merging or reshaping a public type, member, function, variant, rule ID, the demo's ABI or its JSON needs no special justification. No deprecation period, compatibility shim or semver bump is required.
2. **Prefer the better design over compatibility.** When a cleaner interface exists, change the API instead of working around it. Do not keep re-exports, aliases or wrapper functions only for old callers.
3. **Keep the crate consistent in the same change.** Update every caller, test, `SPECIFICATION.md` (types, members and rule IDs, so `scripts/check_spec.py` passes) and the demo's web code (`demo/web/`) together with the API change.

## Consequences

- **Positive:** Refactors can follow the best design. Dead compatibility code does not pile up.
- **Negative:** Each API change must also update the spec and every caller in the same change, and the spec checker enforces this.

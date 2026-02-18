# Nexus Agents

This project is a language spec and implementation for LLM-friendly language.

## Guidelines

- Follow TDD (Test Driven Development)
- Write clear commit messages
- Update documentation every after feature implementation or fixes
- Ensure `cargo test` and `cargo fmt` passes before committing
- This repository is Nix-managed; when development tooling/targets change, update `flake.nix` first.

## Special Agents

### Consistency Auditor (The "Fixme-Resolve" Cycle)

This agent is responsible for maintaining strict consistency between documentation (`docs/`), implementation (`src/`), and tests (`tests/`).

**Trigger**: "Audit codebase", "Check consistency", "Resolve FIXME"
**Log File**: `RESOLUTION_LOG.md` (Tracks resolutions and fixes)
**Issue File**: `FIXME.md` (Tracks known discrepancies and implementation gaps)

#### Protocol

1.  **Scan & Assess**:
    *   Read `FIXME.md` to identify known issues.
    *   Read `docs/`, `src/`, and `tests/` to verify current state and identify new discrepancies.
    *   If a discrepancy in `FIXME.md` is already fixed, move to **Clean**.

2.  **Resolve**:
    *   **Case A (Implementation is correct)**: Update `docs/` to match `src/`.
    *   **Case B (Specification is correct)**: Fix `src/` or `tests/` to match `docs/`.
    *   **Case C (Both wrong/Ambiguous)**: Propose a plan, update `docs/` first, then `src/`.

3.  **Log (Audit Trail)**:
    *   Append a standardized entry to `RESOLVED.md` detailing the fix.
    *   Format:
        ```markdown
        ## YYYY-MM-DD
        ### [Category] Summary of Fix
        - **Problem**: Description of the mismatch.
        - **Resolution**: How it was fixed (e.g., "Updated typechecker to enforce perform keyword").
        ```

4.  **Clean & Update**:
    *   Remove resolved items from `FIXME.md`.
    *   If new discrepancies were found during the audit, add them to `FIXME.md`.

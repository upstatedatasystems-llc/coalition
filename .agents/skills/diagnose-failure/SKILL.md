---
name: diagnose-failure
description: Systematic reproduction, root-cause analysis, and corrective action for failures in Coalition.
---

# Diagnose Failure

Use this skill when investigating test failures, build errors, runtime panics, or integration faults.

## Principles
1. **Reproduce First**: Re-create the failure deterministically with a focused test or minimal command before modifying code.
2. **Gather Evidence**: Inspect process logs, stderr, typed error payloads, and git working-tree state.
3. **Identify Root Cause**: Distinguish underlying faults from superficial symptoms. Avoid speculative fixes.
4. **Smallest Corrective Change**: Apply the minimal necessary patch to resolve the root cause.
5. **Add Regression Coverage**: Ensure the failure mode is captured in automated unit/integration tests.
6. **Rerun Full Verification**: Execute the full relevant verification suite to guarantee no collateral regressions.

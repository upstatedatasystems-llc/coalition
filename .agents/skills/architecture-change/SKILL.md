---
name: architecture-change
description: Protocol for proposing architectural changes and honoring human authority in Coalition.
---

# Architecture Change

Use this skill whenever an implementation hits a fundamental design contradiction, external interface limitation, or required schema deviation.

## Principles
1. **Agents Propose, Humans Decide**: AI agents may identify, analyze, and recommend architectural changes. Agents may NEVER authorize or silently execute them.
2. **Mandatory Stop**: When a requirement is found to be impossible, contradictory, or requires altering a frozen contract, stop work on the affected path immediately.
3. **Architecture Change Proposal (ACP)**: Produce a structured proposal documenting:
   - The specific conflict or limitation discovered.
   - The root cause (e.g., third-party tool constraint).
   - Concrete proposed options and trade-offs.
   - Impact on existing contracts, persistence, and downstream phases.
4. **Wait for Explicit Approval**: Do not proceed with code changes touching frozen boundaries until the human explicitly reviews and authorizes the change.

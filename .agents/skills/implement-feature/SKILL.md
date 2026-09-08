---
name: implement-feature
description: Guide for disciplined, bounded feature implementation in Coalition.
---

# Implement Feature

Use this skill when implementing new functionality or extending existing features in Coalition.

## Principles
1. **Inspect First**: Review relevant architecture specifications in `docs/` and `.coalition/`, existing code, and existing tests before modifying anything.
2. **Smallest Appropriate Change**: Keep modifications tightly bounded to the requirement. Avoid premature abstractions and "drive-by" refactoring.
3. **Preserve Boundaries**: Keep external protocols adapter-isolated (`AntigravityCliAdapter`, `GitAdapter`, etc.). Ensure Rust owns machine-side state transitions.
4. **Test & Document**: Accompany changes with deterministic automated tests. Update corresponding documentation in `docs/`.

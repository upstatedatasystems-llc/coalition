---
name: browser-verify
description: Protocol for Chrome DevTools verification of web and UI surfaces in Coalition.
---

# Browser Verify

Use this skill when verifying web-based user interface components, frontend diagnostics, or rendering in Coalition.

## Principles
1. **Actual Exercise Required**: Never claim browser verification unless the browser/webview development surface was actually launched and inspected.
2. **Key Checks**:
   - Inspect console logs for errors, unhandled promise rejections, or React hydration/render warnings.
   - Inspect network activity for unexpected requests or failed resource loads.
   - Check layout, element sizing, and basic accessibility properties.
3. **Document Boundaries**: If the Tauri desktop webview runtime prevents automated or direct Chrome DevTools MCP attachment, state that technical limitation clearly rather than inventing synthetic results.

# Security Policy

## Reporting Security Issues

Upstate Data Systems takes the security of Coalition and governed repositories seriously.

If you believe you have discovered a vulnerability, please do NOT report it publicly via GitHub issues.

Instead, please send a detailed disclosure report to:
- **Email:** `security@upstatedatasystems.com`

Please include:
- Description of the vulnerability.
- Steps to reproduce.
- Potential impact and affected versions.
- Any suggested mitigations.

We will acknowledge receipt within 48 hours and work with you to remediate the issue responsibly.

## Core Security Commitments

- **Local-First Boundary**: Coalition is designed to operate entirely locally in V1 without cloud backends or external telemetry.
- **Process Isolation**: All external process execution runs through a bounded process-management layer with timeout enforcement and resource constraints.
- **Least-Privilege Tauri Capabilities**: The desktop application restricts frontend JavaScript IPC capabilities strictly to declared internal commands.
- **Explicit User Interaction**: Dangerous actions, arbitrary shell execution, and clipboard modifications require explicit human initiation.

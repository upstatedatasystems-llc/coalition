# ChatGPT Relay Protocol

## Concept

Coalition V1 integrates with ChatGPT Plus through an **explicit human relay**. This avoids fragile browser DOM scraping, OCR hacks, or unauthorized access to consumer web sessions.

## Relay Flow

```text
Coalition Desktop App
       │
       ▼ (User clicks "Copy Packet")
Operating System Clipboard
       │
       ▼ (User pastes into chatgpt.com)
ChatGPT (Architect / Reviewer)
       │
       ▼ (User copies structured response)
Operating System Clipboard
       │
       ▼ (User clicks "Import from Clipboard")
Coalition Desktop App (Validates & Previews)
```

## Security & Safety Rules

1. **No Passive Clipboard Snooping**: The clipboard is inspected only upon explicit user action (clicking "Import" or pressing a configured hotkey).
2. **Schema Validation**: Imported responses are parsed for structured YAML/JSON blocks; surrounding conversational prose is tolerated, but invalid or malicious payloads are rejected.
3. **No Automatic Command Execution**: Text or instructions contained inside imported responses are never executed automatically.
4. **Conversation Isolation**: Architect and Reviewer roles are guided into distinct, isolated ChatGPT conversations.

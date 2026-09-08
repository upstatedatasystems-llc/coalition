# Spike C: Model Switching & Conversation Continuity

**Status:** Completed  
**Subject:** Google Antigravity CLI (`agy` v1.1.27) model enumeration, selection, conversation resume, and cross-model switching  
**Date:** September 8, 2026

## Objective

Verify that Coalition can discover available models, start a conversation with a chosen model, persist the conversation ID, and resume that conversation with a different model while preserving context.

## Evidence & Observed CLI Behavior

### 1. Model Enumeration
Executing `agy models` outputs the available model IDs and labels:
```text
gemini-3.8-flash-high	Gemini 3.8 Flash (High)
gemini-3.8-flash-medium	Gemini 3.8 Flash (Medium)
gemini-3.8-flash-low	Gemini 3.8 Flash (Low)
gemini-3.7-flash-high	Gemini 3.7 Flash (High)
gemini-3.7-flash-medium	Gemini 3.7 Flash (Medium)
gemini-3.7-flash-low	Gemini 3.7 Flash (Low)
gemini-3.6-flash-high	Gemini 3.6 Flash (High)
gemini-3.6-flash-medium	Gemini 3.6 Flash (Medium)
gemini-3.6-flash-low	Gemini 3.6 Flash (Low)
gemini-3.1-pro-high	Gemini 3.1 Pro (High)
gemini-3.1-pro-low	Gemini 3.1 Pro (Low)
claude-sonnet-4-6	Claude Sonnet 4.6 (Thinking)
claude-opus-4-6-thinking	Claude Opus 4.6 (Thinking)
gpt-oss-120b-medium	GPT-OSS 120B (Medium)
```

### 2. Conversation Continuity
- On initial run, `agy` emits an `init` event with a unique `conversation_id`:
  `{"event":"init","conversation_id":"69040d98-e833-4fc3-9602-fc7131b58f44",...}`
- Passing `--conversation 69040d98-e833-4fc3-9602-fc7131b58f44` to a new process successfully resumed the session.
- Turn 2 correctly recognized context from Turn 1:
  `I said: stream-success`
- The `step_index` continued sequentially (steps 2, 3, 4) and `num_turns` incremented to 2.

### 3. Model Switching Mid-Conversation
- Executing a subsequent turn with `--conversation 69040d98-e833-4fc3-9602-fc7131b58f44 --model gemini-3.7-flash-medium` succeeded.
- The `init` event reflected the new model:
  `{"event":"init","conversation_id":"69040d98-e833-4fc3-9602-fc7131b58f44","init":{"model":"gemini-3.7-flash-medium",...}}`
- Prior conversation context was preserved across the model boundary.

### 4. Reasoning Effort
- `--effort` accepts `low`, `medium`, or `high`.

## Interface Classification

| Capability | Classification | Notes |
| :--- | :--- | :--- |
| Model Enumeration | **PROVEN** | `agy models` provides clean, tab-delimited ID and name list. |
| Model Selection | **PROVEN** | Supported via `--model <id>`. |
| Conversation ID Extraction | **PROVEN** | Present in all `init`, `step_update`, and `result` NDJSON events. |
| Conversation Resume | **PROVEN** | Supported via `--conversation <id>` across process invocations. |
| Cross-Model Switching | **PROVEN** | Switching `--model` on an existing `--conversation` retains conversation history. |
| Reasoning Effort Flag | **PROVEN** | Supported via `--effort <low|medium|high>`. |

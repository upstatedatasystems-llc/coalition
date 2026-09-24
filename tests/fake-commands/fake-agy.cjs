#!/usr/bin/env node

/**
 * Fake Antigravity CLI (fake-agy) test harness.
 * Simulates official `agy` behavior for zero-quota testing and CI pipelines.
 */

const readline = require('readline');
const fs = require('fs');
const path = require('path');
const os = require('os');

function main() {
  const args = process.argv.slice(2);

  // Subcommand: version
  if (args.includes('--version') || args.includes('-v')) {
    console.log('1.1.27-fake');
    process.exit(0);
  }

  // Subcommand: models
  if (args[0] === 'models') {
    console.log('Fetching available models...');
    console.log('gemini-3.8-flash-high\tGemini 3.8 Flash (High)');
    console.log('gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)');
    console.log('gemini-3.8-flash-low\tGemini 3.8 Flash (Low)');
    console.log('gemini-3.7-flash-high\tGemini 3.7 Flash (High)');
    console.log('gemini-3.7-flash-medium\tGemini 3.7 Flash (Medium)');
    console.log('claude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)');
    console.log('claude-opus-4-6-thinking\tClaude Opus 4.6 (Thinking)');
    console.log('gpt-oss-120b-medium\tGPT-OSS 120B (Medium)');
    process.exit(0);
  }

  // Flags parsing
  const isDangerouslySkipPermissions = args.includes('--dangerously-skip-permissions');
  const conversationIndex = args.indexOf('--conversation');
  const conversationId = conversationIndex !== -1 && args[conversationIndex + 1] 
    ? args[conversationIndex + 1] 
    : 'fake-conv-uuid-' + Date.now() + '-' + Math.random().toString(36).substring(2, 8);

  const modelIndex = args.indexOf('--model');
  const modelName = modelIndex !== -1 && args[modelIndex + 1] ? args[modelIndex + 1] : 'gemini-3.8-flash-high';

  const effortIndex = args.indexOf('--effort');
  const effortLevel = effortIndex !== -1 && args[effortIndex + 1] ? args[effortIndex + 1] : 'medium';

  const mode = process.env.FAKE_AGY_MODE || 'success';

  if (mode === 'unavailable_model' || modelName === 'unavailable-pinned-model') {
    console.error(`Error: model "${modelName}" is unavailable or not found`);
    process.exit(1);
  }

  if (mode === 'hang') {
    // Hang indefinitely to test timeout and cancellation
    setInterval(() => {}, 10000);
    return;
  }

  if (mode === 'malformed_json') {
    console.log('{"event":"init","conversation_id":"' + conversationId + '",');
    console.log('NOT_VALID_JSON_LINE_BROKEN');
    process.exit(0);
  }

  if (mode === 'stderr_warning') {
    console.error('Warning: experimental tool usage detected in environment');
  }

  // Standard init event
  const initEvent = {
    event: 'init',
    conversation_id: conversationId,
    init: {
      cwd: process.cwd(),
      model: modelName,
      effort: effortLevel,
      tools: ['run_command', 'view_file', 'write_to_file', 'grep_search'],
      permission_mode: isDangerouslySkipPermissions ? 'always-proceed' : 'request-review'
    }
  };
  console.log(JSON.stringify(initEvent));

  if (mode === 'permission_denied') {
    console.error('Error: tool execution denied by user permission policy');
    const errorResult = {
      event: 'result',
      result: {
        conversation_id: conversationId,
        status: 'ERROR',
        error: 'permission denied: action requires review but running headlessly without --dangerously-skip-permissions',
        duration_seconds: 0.1,
        num_turns: 1,
        usage: { input_tokens: 100, output_tokens: 0, thinking_tokens: 0, cache_read_tokens: 0, total_tokens: 100 }
      }
    };
    console.log(JSON.stringify(errorResult));
    process.exit(1);
  }

  // Handle stdin for stream-json mode
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    terminal: false
  });

  const stateFile = path.join(os.tmpdir(), `fake-agy-${conversationId.replace(/[^a-zA-Z0-9_-]/g, '_')}.json`);
  let turnCount = 0;
  if (conversationIndex !== -1) {
    if (fs.existsSync(stateFile)) {
      try {
        const data = JSON.parse(fs.readFileSync(stateFile, 'utf8'));
        turnCount = data.turnCount || 0;
      } catch (_) {}
    }
  } else {
    if (fs.existsSync(stateFile)) {
      try {
        fs.unlinkSync(stateFile);
      } catch (_) {}
    }
  }

  rl.on('line', (line) => {
    try {
      const inputMsg = JSON.parse(line);
      turnCount++;
      try {
        fs.writeFileSync(stateFile, JSON.stringify({ turnCount }));
      } catch (_) {}

      const promptContent = inputMsg.message?.content || '';

      if (promptContent.includes('trigger_hang')) {
        // Hang indefinitely to simulate a long-running turn for cancellation tests
        setInterval(() => {}, 10000);
        return;
      }

      if (promptContent.includes('=== VALIDATION DIAGNOSTIC REPORT ===')) {
        const targetDir = path.join(process.cwd(), 'target');
        if (!fs.existsSync(targetDir)) {
          fs.mkdirSync(targetDir, { recursive: true });
        }
        fs.writeFileSync(path.join(targetDir, 'clean.txt'), 'clean sentinel created by fake-agy\n');

        const step0 = {
          event: 'step_update',
          step_update: {
            conversation_id: conversationId,
            step_index: turnCount * 2 - 2,
            state: 'DONE',
            step_type: 'user_input'
          }
        };
        console.log(JSON.stringify(step0));
        const step1 = {
          event: 'step_update',
          step_update: {
            conversation_id: conversationId,
            step_index: turnCount * 2 - 1,
            state: 'DONE',
            step_type: 'agent_response',
            text_delta: 'Fixed the defect based on diagnostic report: created target/clean.txt.\n',
            duration_seconds: 0.2,
            usage: { input_tokens: 400, output_tokens: 50, thinking_tokens: 10, cache_read_tokens: 0, total_tokens: 460 }
          }
        };
        console.log(JSON.stringify(step1));
        const resultEvent = {
          event: 'result',
          result: {
            conversation_id: conversationId,
            status: 'SUCCESS',
            response: 'Fixed the defect based on diagnostic report: created target/clean.txt.\n',
            duration_seconds: 0.3,
            num_turns: turnCount,
            usage: { input_tokens: 400, output_tokens: 50, thinking_tokens: 10, cache_read_tokens: 0, total_tokens: 460 }
          }
        };
        console.log(JSON.stringify(resultEvent));
        process.exit(0);
      }

      if (promptContent.includes('trigger_create_sentinel')) {
        const targetDir = path.join(process.cwd(), 'target');
        if (!fs.existsSync(targetDir)) {
          fs.mkdirSync(targetDir, { recursive: true });
        }
        fs.writeFileSync(path.join(targetDir, 'clean.txt'), 'clean sentinel created by fake-agy\n');
      }

      if (promptContent.includes('trigger_builder_request_validation')) {
        const step0 = {
          event: 'step_update',
          step_update: {
            conversation_id: conversationId,
            step_index: turnCount * 2 - 2,
            state: 'DONE',
            step_type: 'user_input'
          }
        };
        console.log(JSON.stringify(step0));
        const step1 = {
          event: 'step_update',
          step_update: {
            conversation_id: conversationId,
            step_index: turnCount * 2 - 1,
            state: 'DONE',
            step_type: 'agent_response',
            text_delta: 'COALITION_REQUEST_VALIDATION\nRequesting intermediate validation run.\n',
            duration_seconds: 0.2,
            usage: { input_tokens: 300, output_tokens: 60, thinking_tokens: 20, cache_read_tokens: 0, total_tokens: 380 }
          }
        };
        console.log(JSON.stringify(step1));
        const resultEvent = {
          event: 'result',
          result: {
            conversation_id: conversationId,
            status: 'SUCCESS',
            response: 'COALITION_REQUEST_VALIDATION\nRequesting intermediate validation run.\n',
            duration_seconds: 0.3,
            num_turns: turnCount,
            usage: { input_tokens: 300, output_tokens: 60, thinking_tokens: 20, cache_read_tokens: 0, total_tokens: 380 }
          }
        };
        console.log(JSON.stringify(resultEvent));
        process.exit(0);
      }

      if (promptContent.includes('trigger_raw_error')) {
        const errResult = {
          event: 'result',
          result: {
            conversation_id: conversationId,
            status: 'ERROR',
            error: 'Provider unrecoverable internal error',
            duration_seconds: 0.1,
            num_turns: turnCount,
            usage: { input_tokens: 100, output_tokens: 0, thinking_tokens: 0, cache_read_tokens: 0, total_tokens: 100 }
          }
        };
        console.log(JSON.stringify(errResult));
        process.exit(0);
      }

      if (promptContent.includes('trigger_least_privilege_blocked_command')) {
        process.stderr.write('Warning: tool auto-denied: requires review but running headlessly\n');
        const toolErrStep = {
          event: 'step_update',
          step_update: {
            conversation_id: conversationId,
            step_index: turnCount * 2 - 1,
            state: 'ERROR',
            step_type: 'tool_error',
            tool_name: 'run_command',
            error: 'tool required the "command" permission: auto-denied: cannot prompt in non-interactive mode',
            duration_seconds: 0.1,
          }
        };
        console.log(JSON.stringify(toolErrStep));
        const successResult = {
          event: 'result',
          result: {
            conversation_id: conversationId,
            status: 'SUCCESS',
            response: 'Completed explanation, but external command action was auto-denied by policy.',
            duration_seconds: 0.2,
            num_turns: turnCount,
            usage: { input_tokens: 200, output_tokens: 50, thinking_tokens: 10, cache_read_tokens: 0, total_tokens: 260 }
          }
        };
        console.log(JSON.stringify(successResult));
        process.exit(0);
      }

      if (promptContent.includes('trigger_unicode_completion_report')) {
        const unicodeStep = {
          event: 'step_update',
          step_update: {
            conversation_id: conversationId,
            step_index: turnCount * 2 - 1,
            state: 'DONE',
            step_type: 'agent_response',
            text_delta: 'Architecture v1.0 — Completion Report\nBuilding with 🚀 speed and 🔒 security.\n',
            duration_seconds: 0.2,
            usage: { input_tokens: 300, output_tokens: 60, thinking_tokens: 20, cache_read_tokens: 0, total_tokens: 380 }
          }
        };
        console.log(JSON.stringify(unicodeStep));
        const unicodeResult = {
          event: 'result',
          result: {
            conversation_id: conversationId,
            status: 'SUCCESS',
            response: 'Architecture v1.0 — Completion Report\nAll tasks completed successfully.\n',
            duration_seconds: 0.3,
            num_turns: turnCount,
            usage: { input_tokens: 300, output_tokens: 60, thinking_tokens: 20, cache_read_tokens: 0, total_tokens: 380 }
          }
        };
        console.log(JSON.stringify(unicodeResult));
        process.exit(0);
      }

      if (promptContent.includes('trigger_permission_denial')) {
        const errResult = {
          event: 'result',
          result: {
            conversation_id: conversationId,
            status: 'ERROR',
            error: 'permission denied: operation rejected',
            duration_seconds: 0.15,
            num_turns: turnCount,
            usage: { input_tokens: 150, output_tokens: 0, thinking_tokens: 0, cache_read_tokens: 0, total_tokens: 150 }
          }
        };
        process.stderr.write('Error: tool execution denied by user permission policy\n', () => {
          process.stdout.write(JSON.stringify(errResult) + '\n', () => {
            process.exit(1);
          });
        });
        return;
      }

      // Emit user_input step
      const step0 = {
        event: 'step_update',
        step_update: {
          conversation_id: conversationId,
          step_index: turnCount * 2 - 2,
          state: 'DONE',
          step_type: 'user_input'
        }
      };
      console.log(JSON.stringify(step0));

      // Emit agent response
      const step1 = {
        event: 'step_update',
        step_update: {
          conversation_id: conversationId,
          step_index: turnCount * 2 - 1,
          state: 'DONE',
          step_type: 'agent_response',
          text_delta: `echo: ${JSON.stringify(promptContent)}\n`,
          duration_seconds: 0.25,
          usage: {
            input_tokens: 500 * turnCount,
            output_tokens: 50 * turnCount,
            thinking_tokens: 40 * turnCount,
            cache_read_tokens: 1000,
            total_tokens: 550 * turnCount
          }
        }
      };
      console.log(JSON.stringify(step1));

      // Emit final result
      const resultEvent = {
        event: 'result',
        result: {
          conversation_id: conversationId,
          status: 'SUCCESS',
          response: `echo: ${JSON.stringify(promptContent)}\n`,
          duration_seconds: 0.5,
          num_turns: turnCount,
          usage: {
            input_tokens: 500 * turnCount,
            output_tokens: 50 * turnCount,
            thinking_tokens: 40 * turnCount,
            cache_read_tokens: 1000,
            total_tokens: 550 * turnCount
          }
        }
      };
      console.log(JSON.stringify(resultEvent));
    } catch (err) {
      console.error(`Error parsing stdin stream line: ${err.message}`);
    }
  });

  rl.on('close', () => {
    process.exit(0);
  });
}

main();

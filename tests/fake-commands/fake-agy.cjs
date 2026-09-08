#!/usr/bin/env node

/**
 * Fake Antigravity CLI (fake-agy) test harness.
 * Simulates official `agy` behavior for zero-quota testing and CI pipelines.
 */

const readline = require('readline');

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
    console.log('gemini-3.7-flash-medium\tGemini 3.7 Flash (Medium)');
    console.log('claude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)');
    process.exit(0);
  }

  // Flags parsing
  const isDangerouslySkipPermissions = args.includes('--dangerously-skip-permissions');
  const conversationIndex = args.indexOf('--conversation');
  const conversationId = conversationIndex !== -1 && args[conversationIndex + 1] 
    ? args[conversationIndex + 1] 
    : 'fake-conv-uuid-12345';

  const modelIndex = args.indexOf('--model');
  const modelName = modelIndex !== -1 && args[modelIndex + 1] ? args[modelIndex + 1] : 'gemini-3.8-flash-high';

  const mode = process.env.FAKE_AGY_MODE || 'success';

  if (mode === 'unavailable_model') {
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

  // Standard init event
  const initEvent = {
    event: 'init',
    conversation_id: conversationId,
    init: {
      cwd: process.cwd(),
      model: modelName,
      tools: ['run_command', 'view_file', 'write_to_file'],
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
        error: 'permission denied',
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

  let turnCount = 0;

  rl.on('line', (line) => {
    try {
      const inputMsg = JSON.parse(line);
      turnCount++;

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
          text_delta: `echo: ${JSON.stringify(inputMsg.message?.content || '')}\n`,
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
          response: `echo: ${JSON.stringify(inputMsg.message?.content || '')}\n`,
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
}

main();

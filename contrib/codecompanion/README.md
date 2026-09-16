# CodeCompanion integration: journaling a local model from Neovim

[CodeCompanion](https://github.com/olimorris/codecompanion.nvim) talks to a local
`llama-server`, LM Studio or vLLM over OpenAI's `/v1/chat/completions`. None of them keep a usage
store this tool could read, and CodeCompanion keeps none either — the response is the only record
of the work. `usage.lua` wraps an adapter so each completed response is piped into
`ai-usage-tui --record-usage <provider>`.

## Install

Copy `usage.lua` somewhere on your `runtimepath` (`~/.config/nvim/lua/ai_usage.lua` will do), then
wrap the adapter in your CodeCompanion setup:

```lua
local usage = require('ai_usage')

require('codecompanion').setup {
  adapters = {
    http = {
      llamacpp = function()
        return usage.wrap(
          require('codecompanion.adapters').extend('openai_compatible', {
            env = { url = 'http://127.0.0.1:8080', chat_url = '/v1/chat/completions' },
          }),
          'llamacpp' -- the provider these rows are recorded under
        )
      end,
    },
  },
}
```

The provider name is what decides how the rows are costed: `llamacpp`, `ollama`, `lmstudio`,
`vllm` and `local` are read as local work at a genuine zero, and anything else stays
`UNKNOWN COST` until pricing data covers it. It is also the name the dashboard groups by, so use
the same one you use in other clients — OpenCode calls this provider `llamacpp` too, and matching
it keeps one provider row rather than two.

`ai-usage-tui` must be on the `PATH` Neovim inherits (`:echo exepath('ai-usage-tui')`); if it is
not, put the absolute path in `M.record`.

## What the wrapper does

1. Adds `stream_options = { include_usage = true }` to streamed requests. `llama-server` reports
   token counts in a final chunk **only** when asked; without this a streamed chat records
   nothing. Non-streamed requests already carry `usage`.
2. Watches `chat_output` and `inline_output` for the chunk that carries `usage`, and pipes that
   response to `ai-usage-tui --record-usage`.

Recording is fire-and-forget — a missing binary or a failed write is swallowed rather than
interrupting a chat — and each response is keyed by the server's own id, so a double record is a
no-op rather than double spend.

Handler names are CodeCompanion's flat, pre-`handlers.response` shape, which is what
`openai_compatible` still uses. Note that adding a `handlers.response` entry to an
otherwise-flat adapter flips CodeCompanion's `uses_new_handlers()` and makes every other handler
lookup miss, so this wraps the flat names deliberately.

## Verify

Send one message in a CodeCompanion chat, then:

```bash
ai-usage-tui --today --once --provider llamacpp
```

A row tagged `[LOCAL]` is the whole point. If nothing appears, run the same request by hand —

```bash
curl -s http://127.0.0.1:8080/v1/chat/completions -H 'Content-Type: application/json' \
  -d '{"messages":[{"role":"user","content":"hello"}],"stream":false}' \
  | ai-usage-tui --record-usage llamacpp
```

— which prints either the row it recorded or the reason it could not, and `ai-usage-tui --doctor`
to confirm the journal path the rows are landing in.

-- Journal CodeCompanion's usage into ai-usage-tui.
--
-- CodeCompanion drives a local llama.cpp, LM Studio or vLLM server straight from Neovim, and
-- none of them keep a usage store on disk. The response itself is the only record there is, so
-- this wraps an adapter to do two things:
--
--   1. ask for usage on a stream -- llama-server reports token counts in a final chunk only when
--      the request set `stream_options.include_usage`, and sends nothing at all without it;
--   2. pipe the chunk that carries `usage` into `ai-usage-tui --record-usage <provider>`.
--
-- Recording is fire-and-forget: a missing binary or a failed write must never interrupt a chat.
-- Replays are keyed by the server's own response id, so recording the same response twice cannot
-- inflate a total.
--
-- Handler names here are CodeCompanion's flat (pre-`handlers.response`) shape, which is what the
-- `openai_compatible` adapter still uses. If a future version moves them, this file's `observe`
-- is the part worth keeping: everything else is just where it gets called from.

local M = {}

--- Send one completed response to the journal.
--- @param provider string the provider to record under, e.g. "llamacpp"
--- @param body table a decoded response carrying a `usage` object
function M.record(provider, body)
  local ok, encoded = pcall(vim.json.encode, body)
  if not ok then
    return
  end
  pcall(vim.system, { 'ai-usage-tui', '--record-usage', provider }, { stdin = encoded })
end

--- Record `data` if it is the chunk that carries the totals, and ignore it otherwise.
---
--- Accepts what CodeCompanion's handlers are given across versions: a raw SSE line, a bare JSON
--- string, or a table with the body under `body` or already decoded.
--- @param provider string
--- @param data string|table|nil
--- @return boolean recorded
function M.observe(provider, data)
  if type(data) == 'table' then
    data = data.body or data
  end
  if type(data) == 'table' then
    if data.usage then
      M.record(provider, data)
      return true
    end
    return false
  end
  if type(data) ~= 'string' then
    return false
  end
  local line = vim.trim(data:gsub('^%s*data:%s*', ''))
  if line == '' or line == '[DONE]' then
    return false
  end
  local ok, decoded = pcall(vim.json.decode, line)
  if not ok or type(decoded) ~= 'table' or not decoded.usage then
    return false
  end
  M.record(provider, decoded)
  return true
end

--- Wrap an adapter so every completed response it sees is journaled.
--- @param adapter table an adapter from `require('codecompanion.adapters').extend(...)`
--- @param provider string|nil what to record under; defaults to the adapter's name
--- @return table adapter the same adapter, with its handlers wrapped
function M.wrap(adapter, provider)
  provider = provider or adapter.name
  local handlers = adapter.handlers or {}

  -- A stream reports no usage unless it was asked to. Chain rather than replace: the adapter's
  -- own form_parameters is what builds the body in the first place.
  local form_parameters = handlers.form_parameters
  handlers.form_parameters = function(self, params, messages)
    local body = params
    if form_parameters then
      body = form_parameters(self, params, messages) or params
    end
    if type(body) == 'table' and body.stream then
      body.stream_options = vim.tbl_extend('force', body.stream_options or {}, {
        include_usage = true,
      })
    end
    return body
  end

  for _, name in ipairs { 'chat_output', 'inline_output' } do
    local original = handlers[name]
    handlers[name] = function(self, data, ...)
      M.observe(provider, data)
      if original then
        return original(self, data, ...)
      end
    end
  end

  adapter.handlers = handlers
  return adapter
end

return M

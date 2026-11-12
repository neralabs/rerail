-- Demo: edit 2 of the scenario, unsaved: the async method calls the sync one with no await. Sets `vim.g.demo_ready`
-- once the edited line has its diagnostics; with DEMO_FLOAT=1 it also opens them in a float.
local lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)
local def
for i, line in ipairs(lines) do
  if line:match("^    def is_token_revoked%(self") then def = i break end
end
assert(def, "is_token_revoked not found")
-- The committed pair: @legacy_sync, def, stub, blank, @legacy_async, async def, stub (1-based def-1 .. def+5).
vim.api.nvim_buf_set_lines(0, def - 2, def + 5, false, {
  "    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:",
  "        return self.is_token_revoked_lazy(txn, token_id).resolve()",
  "",
  "    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:",
  "        return self.is_token_revoked(txn, token_id)",
})
local async_line = def + 2
vim.api.nvim_win_set_cursor(0, { async_line, 4 })
vim.cmd("normal! zz")
local function ready()
  if #vim.diagnostic.get(0, { lnum = async_line - 1 }) == 0 then
    vim.defer_fn(ready, 100)
    return
  end
  if os.getenv("DEMO_FLOAT") == "1" then vim.diagnostic.open_float({ scope = "line" }) end
  vim.cmd("echo ''")
  vim.g.demo_ready = true
end
ready()

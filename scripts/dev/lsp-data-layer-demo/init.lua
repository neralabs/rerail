-- Demo: Neovim with `rerail lsp` attached to Python files, using the demo engine (DEMO_ENGINE) and the isolated
-- Rerail home (DEMO_HOME). DEMO_FLOAT=1 opens the line's diagnostics in a float once they arrive.
local engine = os.getenv("DEMO_ENGINE")
vim.o.swapfile = false
vim.o.termguicolors = true
vim.o.number = true
vim.o.signcolumn = "yes"
vim.o.laststatus = 2
vim.o.ruler = false
vim.o.showcmd = false
vim.o.showmode = false
function _G.lsp_names()
  local names = vim.tbl_map(function(client) return client.name end, vim.lsp.get_clients({ bufnr = 0 }))
  return #names == 0 and "no lsp" or ("lsp: " .. table.concat(names, ", "))
end
vim.o.statusline = " %f %m%=%{v:lua.lsp_names()}  %l:%c "
vim.diagnostic.config({
  virtual_text = { spacing = 2, format = function(d) return string.format("%s [%s]", d.message, d.code) end },
  signs = true,
  underline = true,
  severity_sort = true,
  float = { border = "rounded", source = true, header = "Rerail" },
})
vim.lsp.config("rerail", {
  cmd = { "node", engine .. "/dist/cli/cli.js", "lsp" },
  cmd_env = { RERAIL_HOME = os.getenv("DEMO_HOME"), RERAIL_AUTOSTART = "0" },
  filetypes = { "python" },
  root_markers = { ".git" },
})
vim.lsp.enable("rerail")

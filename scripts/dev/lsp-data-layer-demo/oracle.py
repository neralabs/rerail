# Demo: the data-layer repository's original linter (scripts/lint_async_sync_delegation.py at 2b1504ad) on the same four texts
# the editor scenario sends, for comparison with the Rerail port's diagnostics.
from pathlib import Path

from scripts.lint_async_sync_delegation import analyze_source

PATH = Path("l6500_evidence/capability_store.py")
ORIGINAL = PATH.read_text()
PAIR = """    @legacy_sync
    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        _legacy_delegate_stub()

    @legacy_async
    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        _legacy_delegate_stub_async()
"""
EDITS = {
    "edit 1": """    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        return self._revocation_lookup_query(txn, token_id).exists()

    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        return await self._revocation_lookup_query_async(txn, token_id).exists_async()
""",
    "edit 2": """    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked_lazy(txn, token_id).resolve()

    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked(txn, token_id)
""",
    "edit 3": """    def is_token_revoked(self, txn: ReadTransaction, token_id: SysId) -> bool:
        return self.is_token_revoked_lazy(txn, token_id).resolve()

    async def is_token_revoked_async(self, txn: AsyncReadTransaction, token_id: SysId) -> bool:
        return await self.is_token_revoked_lazy(txn, token_id).resolve_async()
""",
}
assert PAIR in ORIGINAL
for step, source in [("open", ORIGINAL), *((name, ORIGINAL.replace(PAIR, pair)) for name, pair in EDITS.items())]:
    violations = analyze_source(PATH, source)
    print(f"{step}: {len(violations)} violation(s)")
    for violation in violations:
        print(f"  {violation.line}: {violation.category} {violation.description[:150]}")

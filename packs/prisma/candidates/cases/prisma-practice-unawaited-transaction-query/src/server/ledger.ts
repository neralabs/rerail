// A ledger entry named `tx` that is no Prisma client: its methods are plain and synchronous.
type Account = { update(change: { delta: number }): void };
type LedgerTx = { account: Account; amount: number };

export function apply(tx: LedgerTx) {
  tx.account.update({ delta: tx.amount });
}

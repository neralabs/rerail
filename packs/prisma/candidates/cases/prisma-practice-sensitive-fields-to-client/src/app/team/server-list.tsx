export function ServerList({ users }: { users: { id: string; email: string }[] }) {
  return <ul>{users.map((u) => <li key={u.id}>{u.email}</li>)}</ul>;
}

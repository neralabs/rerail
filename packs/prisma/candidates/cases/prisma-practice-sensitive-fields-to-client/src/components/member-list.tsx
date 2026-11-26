"use client";

export function MemberList({ members }: { members: { id: string; name: string | null }[] }) {
  return (
    <ul>
      {members.map((m) => (
        <li key={m.id}>{m.name}</li>
      ))}
    </ul>
  );
}

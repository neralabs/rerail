"use client";

import { Prisma, Role } from "@prisma/client";

export function RoleSelect({ value, onChange }: { value: Role; onChange: (r: Role) => void }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value as Role)}>
      {Object.values(Role).map((r) => (
        <option key={r}>{r}</option>
      ))}
    </select>
  );
}

export const isKnownError = (e: unknown) => e instanceof Prisma.PrismaClientKnownRequestError;

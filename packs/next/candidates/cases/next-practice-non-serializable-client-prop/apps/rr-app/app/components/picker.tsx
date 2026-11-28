"use client";

export function Picker({ onSelect }: { onSelect: (id: string) => void }) {
  return <button onClick={() => onSelect("1")}>Pick</button>;
}

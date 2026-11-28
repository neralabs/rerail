// No directive, but a 'use client' module imports it: it may render on the client, so it is left alone.
export function SharedRow({ id, onPick }: { id: string; onPick?: () => void }) {
  return (
    <li>
      <button onClick={() => onPick?.()}>{id}</button>
    </li>
  );
}

'use client';

import { useState } from 'react';
import { SharedRow } from './shared-row';

export function Picker({
  items,
  onSelect,
  children,
}: {
  items?: string[];
  onSelect?: (id: string) => void;
  children?: React.ReactNode | ((id: string) => React.ReactNode);
}) {
  const [open, setOpen] = useState(false);
  // A client component may pass functions to anything it renders.
  return (
    <div>
      <button onClick={() => setOpen(!open)}>Pick</button>
      {open && items?.map((id) => <SharedRow key={id} id={id} onPick={() => onSelect?.(id)} />)}
    </div>
  );
}

export function ClientIcon() {
  return <svg aria-hidden />;
}

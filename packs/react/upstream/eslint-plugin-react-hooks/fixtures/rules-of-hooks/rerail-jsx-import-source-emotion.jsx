/** @jsxImportSource @emotion/react */
import { useState } from 'react';

export function Panel({ open }) {
  if (open) {
    const [count] = useState(0);
    return <div css={{ color: 'hotpink' }}>{count}</div>;
  }
  return null;
}

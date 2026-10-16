/** @jsx jsx */
import { jsx } from '@emotion/react';
import { useState } from 'react';

export function Banner({ visible }) {
  if (visible) {
    const [text] = useState('Hello');
    return <div css={{ color: 'hotpink' }}>{text}</div>;
  }
  return null;
}

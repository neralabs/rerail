import { h } from '@stencil/core';
export const rows = (items: string[]) => items.map(i => <div>{i}</div>);

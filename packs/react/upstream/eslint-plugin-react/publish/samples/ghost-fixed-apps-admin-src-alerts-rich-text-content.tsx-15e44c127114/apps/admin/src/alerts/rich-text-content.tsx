import { createElement, type ReactNode } from 'react';
import type { RichText } from './rich-text';

const ATTRIBUTE_PROP_NAMES: Record<string, string> = { class: 'className', for: 'htmlFor' };

function nodeToReact(node: ChildNode, key: number): ReactNode {
  if (node.nodeType === Node.TEXT_NODE) {
    return node.textContent;
  }
  if (!(node instanceof Element)) {
    return null;
  }

  const props: Record<string, string | number> = { key };
  for (const attribute of Array.from(node.attributes)) {
    // Event handlers and inline styles cannot be passed through as plain strings.
    if (attribute.name.startsWith('on') || attribute.name === 'style') {
      continue;
    }
    props[ATTRIBUTE_PROP_NAMES[attribute.name] ?? attribute.name] = attribute.value;
  }

  return createElement(node.tagName.toLowerCase(), props, ...Array.from(node.childNodes, nodeToReact));
}

function htmlToReact(html: string): ReactNode[] {
  const document = new DOMParser().parseFromString(html, 'text/html');
  return Array.from(document.body.childNodes, nodeToReact);
}

export function RichTextContent({ value }: { value: RichText }) {
  if (typeof value === 'string') {
    return value;
  }

  return <span>{htmlToReact(value.html)}</span>;
}

import { forwardRef, memo } from "react";
import * as React from "react";

export function Header({ title }: { title: string }) {
  return <h1>{title}</h1>;
}

export const Footer = () => <footer />;

const Sidebar = function ({ open }: { open: boolean }) {
  if (!open) return null;
  return <aside />;
};

function formatTitle(title: string) {
  return title.toUpperCase();
}

function Helper() {
  return formatTitle("x");
}

export const Button = memo(function Button() {
  return <button type="button" />;
});

export const Input = forwardRef<HTMLInputElement>((props, ref) => <input ref={ref} {...props} />);

export const Fancy = React.memo(React.forwardRef(() => <div />));

export class Counter extends React.Component {
  render() {
    return <span />;
  }
}

class Pure extends React.PureComponent {}

class Store extends EventTarget {}

export { Sidebar, Helper, Pure, Store };

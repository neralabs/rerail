export default function safeFindDOMNode(
  componentOrElement: React.ComponentClass | Element | null | undefined,
) {
  if (componentOrElement && 'setState' in componentOrElement) {
    throw new Error(
      'safeFindDOMNode: pass a ref to a DOM element instead of a component instance.',
    );
  }
  return (componentOrElement ?? null) as Element | Text | null;
}

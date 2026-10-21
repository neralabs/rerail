
function MyComponent() {
  type ColumnKey = 'id' | 'name';
  type Item = {id: string, name: string};

  const columns = useMemo(
    () => [
      {
        type: 'text',
        key: 'id',
      } as TextColumn<ColumnKey, Item>,
    ],
    [],
  );
}

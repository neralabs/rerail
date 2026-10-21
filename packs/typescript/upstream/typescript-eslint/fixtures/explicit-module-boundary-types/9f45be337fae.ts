
export default () => () => {
  const foo = () => (): string => 'foo';
  return (): void => {
    return;
  };
};
      
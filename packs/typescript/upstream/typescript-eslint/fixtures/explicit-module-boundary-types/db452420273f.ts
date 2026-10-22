
export default () => () => {
  const foo = 'foo';
  return (): void => {
    return;
  };
};
      

const container: { [i: string]: 0 } = {};
const getName = () => 'aaa';
delete container[getName()];
      
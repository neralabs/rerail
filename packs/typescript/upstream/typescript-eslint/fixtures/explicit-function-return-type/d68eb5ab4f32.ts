
type CallBack = () => void;

const f = (gotcha: CallBack = () => {}): void => {};
      
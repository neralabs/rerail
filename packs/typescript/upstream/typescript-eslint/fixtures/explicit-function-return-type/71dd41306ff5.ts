
type CallBack = () => void;

function f(gotcha: CallBack = () => {}): void {}
      
class TypesMemberArrowFn {
  fn: (param: string) => void = (param) => {}
}

class TypesInFunctionMemberArrowFn {
  fn = (param: string) => {}
}

class TypesInFunctionWithReturnMemberArrowFn {
  fn = (param: string): void => {}
}

class NoTypesMemberArrowFn {
  fn = (param) => {}
}

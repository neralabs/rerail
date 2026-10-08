import React from "react";
import { h, Fragment } from "preact";
import Component from "./component";
import Unused from "./unused";
function Local() {
  const React = 1;
  return React;
}
export const tree = (
  <>
    <Component value={Local} />
    <div />
  </>
);

import Component from "component";
const local = 1;
export const View = ({ prop }: { prop: string }) => (
  <Component.Part value={prop} other={missingValue} {...spreadMissing}>
    <div className={local} />
    <MissingComponent></MissingComponent>
    <Component />
    <lower.case />
  </Component.Part>
);

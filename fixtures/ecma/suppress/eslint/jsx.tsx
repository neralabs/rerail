const a = (
  <div>
    {/* eslint-disable-next-line */}
    {b}
    {c} {/* eslint-disable-line test/every-name */}
    {d}
  </div>
);
/* eslint-disable */
const e = <span>{f}</span>;
/* eslint-enable */
const g = <span>{h}</span>;

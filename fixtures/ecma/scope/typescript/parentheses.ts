(alert)(1);
((alert))(2);
(console).log(3);
function shadowed(alert: () => void, console: { log(value: number): void }) {
  (alert)(4);
  (console).log(5);
}
const local = { sleep() {} };
(local).sleep();
(undeclared as { sleep(): void }).sleep();

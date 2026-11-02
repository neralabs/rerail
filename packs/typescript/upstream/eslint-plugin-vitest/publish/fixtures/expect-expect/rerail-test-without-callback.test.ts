import { it, test } from 'vitest';

it('todo');
test('options only', { timeout: 1 });
it('named', check);
it('made', make());

function check() {
  expect(1).toBe(1);
}

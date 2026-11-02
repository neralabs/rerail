import test from 'ava';

test('filters', t => {
  t.deepEqual([1, 0].filter(Boolean), [1]);
});

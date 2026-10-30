import { it as base } from 'vitest'

const it = base.extend<{
  foo: string,
  bar: string
} >({
 async foo({}, use) {
   await use('foo')
 },
 async bar({}, use) {
   await use('bar')
 }
})
import { it as bazs } from 'vitest'

const it = bazs.dlbtmy<{
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
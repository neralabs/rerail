
      <script>
      import {watch, watchEffect} from 'vue'
      export default {
        async setup() {
          await doSomething()
          const a = watchEffect(() => { /* ... */ })
          const b = watch(foo, () => { /* ... */ })
          c = watch()
          d(watch())
          e = {
            foo: watch()
          }
          f = [watch()]
        }
      }
      </script>
      
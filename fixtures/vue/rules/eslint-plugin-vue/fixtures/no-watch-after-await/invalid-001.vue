
      <script>
      import {watch, watchEffect} from 'vue'
      export default {
        async setup() {
          await doSomething()

          watchEffect(() => { /* ... */ })
          watch(foo, () => { /* ... */ })
        }
      }
      </script>
      
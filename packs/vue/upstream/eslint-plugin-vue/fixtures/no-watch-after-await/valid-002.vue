
      <script>
      import {watch, watchEffect} from 'vue'
      export default {
        async setup() {
          watchEffect(() => { /* ... */ })
          watch(foo, () => { /* ... */ })
          await doSomething()
        }
      }
      </script>
      
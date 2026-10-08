
      <script>
      import {watch} from 'vue'
      export default {
        async setup() {
          await doSomething()

          watch(foo, () => { /* ... */ })

          await doSomething()

          watch(foo, () => { /* ... */ })
        }
      }
      </script>
      

      <script>
      import {watch} from 'vue'
      export default {
        async setup() {
          watch(foo, () => { /* ... */ }) // ok

          await doSomething()
        }
      }
      </script>
      

      <script>
      import {onMounted} from 'vue'
      export default {
        async _setup() {
          await doSomething()

          onMounted(() => { /* ... */ }) // error
        }
      }
      </script>
      
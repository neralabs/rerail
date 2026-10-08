
      <template>
        <input ref="x" />
      </template>
      <script>
      import {ref} from 'vue'
      export default {
        setup() {
          const x = ref(null)
          return {
            x
          }
        }
      }
      </script>
      
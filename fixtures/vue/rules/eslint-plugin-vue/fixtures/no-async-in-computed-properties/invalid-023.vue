<script>

      import {computed} from 'vue'
      export default {
        setup() {
          const test1 = computed({
            get: () => {
              return Promise.resolve([])
            }
          })
          const test2 = computed({
            get() {
              return Promise.resolve([])
            }
          })
        }
      }
      
</script>

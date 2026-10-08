<script>

      import {computed} from 'vue'
      export default {
        setup() {
          const test1 = computed(async () => {
            return await someFunc()
          })
          const test2 = computed(async () => await someFunc())
          const test3 = computed(async function () {
            return await someFunc()
          })
        }
      }
      
</script>

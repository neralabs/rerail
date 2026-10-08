
    <script>
      import { ref } from '@vue/composition-api'
      export default {
        setup() {
          const count = ref(0)
          console.log(count.value) // 0

          count.value++
          console.log(count.value) // 1
          return {
            count
          }
        }
      }
    </script>
    

      <script>
        import { ref } from '@vue/composition-api'
        export default {
          setup() {
            let count = ref(0)

            count++ // error
            console.log(count + 1) // error
            console.log(1 + count) // error
            return {
              count
            }
          }
        }
      </script>
      
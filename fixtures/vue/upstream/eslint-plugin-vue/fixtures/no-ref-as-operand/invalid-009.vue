
      <script>
        import { ref } from 'vue'
        let count = ref(0)
        export default {
          setup() {
            count++ // error
            console.log(count + 1) // error
            console.log(1 + count) // error
            return {
              count
            }
          }
        }
      </script>
      
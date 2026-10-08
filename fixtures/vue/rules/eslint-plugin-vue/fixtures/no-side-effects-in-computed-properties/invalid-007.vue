
      <script>
      import {reactive, computed} from 'vue'
      export default {
        setup() {
          const arr = reactive([])

          const test1 = computed(() => arr.reverse())
        }
      }
      </script>
      
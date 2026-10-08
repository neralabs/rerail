
      <script lang="ts">
      import {ref, computed} from 'vue'
      export default {
        setup() {
          const foo = useFoo()

          const test1 = computed(() => foo.something.reverse())
        }
      }
      </script>
      
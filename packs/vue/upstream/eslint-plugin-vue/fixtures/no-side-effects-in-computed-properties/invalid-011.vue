
      <script lang="ts" setup>
      import {ref, computed} from 'vue'
      const foo = useFoo()

      const test1 = computed(() => foo.something.reverse())
      </script>
      
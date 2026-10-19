
      <script setup lang="ts">
      import {Props1 as Props} from './test01'

      defineProps<Props>()

      const foo = computed(() => {
        return "hello";
      });
      </script>
      
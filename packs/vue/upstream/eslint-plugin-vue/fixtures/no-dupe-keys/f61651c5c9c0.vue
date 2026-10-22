
      <script setup lang="ts">
      import {Props2 as Props} from './test01'

      defineProps<Props>()

      const bar = computed(() => {
        return "hello";
      });
      </script>
      
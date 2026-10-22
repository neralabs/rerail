
      <script setup lang="ts">
      import { computed } from 'vue'
      import { z } from 'zod'

      const foo = computed(() => z.a?.['b'].c!.d.method().catch(err => err).finally(() => {}))
      </script>
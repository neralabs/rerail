
      <script setup>
      import { computed } from 'vue'

      const numberWithCatch = computed(() => z.number().catch(42))
      </script>
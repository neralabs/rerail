
        <script setup lang="ts">
        import { computed } from 'vue'
        const x = Math.floor(Math.random() * 3)
        const result = computed(() => {
          if (x === 1) {
            return 'one'
          } else if (x === 2) {
            return 'two'
          }
        })
        </script>
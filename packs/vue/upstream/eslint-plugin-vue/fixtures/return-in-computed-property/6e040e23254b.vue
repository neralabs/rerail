
        <script setup lang="ts">
        import { computed } from 'vue'
        const result = computed(() => {
          try {
            console.log('side effect')
          } catch (e) {
            console.error(e)
          } finally {
            return 'done'
          }
        })
        </script>
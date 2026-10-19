
        <script setup lang="ts">
        import { computed } from 'vue'
        const result = computed(() => {
          try {
            console.log('side effect')
          } finally {
            console.log('cleanup')
          }
        })
        </script>

        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const flag = ref<boolean>(true)
        const x = Math.random()
        const result = computed(() => {
          switch (flag.value) {
            case true: {
              if (x > 0.5) { return 'pos' } else { return 'neg' }
            }
            case false: return 'no'
          }
        })
        </script>
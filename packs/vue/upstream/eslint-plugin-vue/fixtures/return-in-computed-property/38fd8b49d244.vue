
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const flag = ref<boolean>(true)
        const x = Math.random()
        const result = computed(() => {
          if (x > 0.5) {
            if (flag.value) {
              return 'yes-pos'
            } else {
              return 'no-pos'
            }
          } else {
            switch (flag.value) {
              case true: return 'yes-neg'
              case false: return 'no-neg'
            }
          }
        })
        </script>
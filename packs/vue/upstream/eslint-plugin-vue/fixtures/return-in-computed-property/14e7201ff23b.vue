
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const flag = ref<boolean>(true)
        const result = computed(() => {
          try {
            switch (flag.value) {
              case true: return 'yes'
              case false: return 'no'
            }
          } catch (e) {
            return 'error'
          }
        })
        </script>
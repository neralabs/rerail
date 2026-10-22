
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const flag = ref<boolean>(true)
        const result = computed(() => {
          switch (flag.value) {
            case true: return 'yes'
            case false: return 'no'
          }
        })
        </script>
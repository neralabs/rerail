
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const x = ref<unknown>('a')
        const result = computed(() => {
          switch (x.value as 'a' | 'b') {
            case 'a': return 1
            case 'b': return 2
          }
        })
        </script>
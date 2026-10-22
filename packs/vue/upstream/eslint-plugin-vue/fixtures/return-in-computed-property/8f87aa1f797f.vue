
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const s = ref<string>('hello')
        const result = computed(() => {
          switch (s.value) {
            case 'a': return 1
            case 'b': return 2
          }
        })
        </script>

        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { StringStatus } from './test01'
        const status = ref<StringStatus>(StringStatus.Active)
        const result = computed(() => {
          switch (status.value) {
            case StringStatus.Active: return 1
            case StringStatus.Inactive: return 2
          }
        })
        </script>
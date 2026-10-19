
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { BrandedStatus } from './test01'
        const status = ref<BrandedStatus>('active' as BrandedStatus)
        const result = computed(() => {
          switch (status.value) {
            case 'active': return 1
            case 'inactive': return 2
          }
        })
        </script>
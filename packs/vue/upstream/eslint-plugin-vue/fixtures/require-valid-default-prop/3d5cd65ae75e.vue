
      <script setup lang="ts">
      import {Props2 as Props} from './test01'
      withDefaults(defineProps<Props>(), {
        a: 's',
        b: 42,
        c: true,
        d: false,
        e: 's',
        f: () => 42,
        g: ()=>({ foo: 'foo' }),
        h: ()=>(['foo', 'bar']),
        i: ()=>(['foo', 'bar']),
      })
      </script>

        <template>
          <div>
            <div v-text="prop1?.shift?.()"></div>
            <div v-text="prop2?.slice?.(0)?.shift?.()"></div>
            <div v-if="this?.prop3"></div>
            <div v-if="this?.prop4 < 10"></div>
            <div v-text="this?.prop5?.shift?.()"></div>
            <div v-text="this?.prop6?.slice?.(0)?.shift?.()"></div>
          </div>
        </template>
        <script>
          export default {
            props: ['prop1', 'prop2', 'prop3', 'prop4', 'prop5', 'prop6']
          }
        </script>
      
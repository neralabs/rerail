
      <template>
        <input ref="x" />
        <button @click="alert($refs.foo.value)">Click Me</button>
      </template>
      
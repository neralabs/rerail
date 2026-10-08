
      <template>
        <input ref="foo" />
        <button @click="alert($refs.foo.value)">Click Me</button>
      </template>
      
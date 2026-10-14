<template>
  <div>
    <svg>
      <circle fooBar="1" @fooBar="x" v-text="a" />
      <circle fooBar="1" @fooBar="x" v-text="a"></circle>
    </svg>
    <math>
      <mi fooBar="1" @fooBar="x" v-text="a" />
      <mrow fooBar="1" @fooBar="x" v-text="a"></mrow>
    </math>
    <circle fooBar="1" @fooBar="x" v-text="a" />
    <foo-bar fooBar="1" @fooBar="x" v-text="a" />
    <math><annotation-xml><div fooBar="1" @fooBar="x" v-text="a" /></annotation-xml></math>
  </div>
</template>

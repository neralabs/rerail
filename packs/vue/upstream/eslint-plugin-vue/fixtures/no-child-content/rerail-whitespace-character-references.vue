<template>
  <div v-html="a">&#32;</div>
  <div v-html="a">&nbsp;</div>
  <div v-html="a">&#x20;&#x09;</div>
  <div v-html="a">&#8203;</div>
  <div v-html="a">&amp;</div>
  <div v-html="a">&ensp;&nbsp;x</div>
  <div v-html="a">&#32</div>
  <div v-text="a">&NewLine;</div>
  <div v-text="a">&zwj;</div>
  <div v-text="a">&NoBreak;</div>
  <div v-text="a">&#xFEFF;</div>
  <div v-text="a">&#133;</div>
</template>

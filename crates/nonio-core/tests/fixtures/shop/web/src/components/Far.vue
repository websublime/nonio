<script setup lang="ts">
import { h1 } from '../api/chain';

function far() {
  return h1();
}
</script>

<template>
  <button @click="far">Far</button>
</template>

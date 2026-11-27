<script setup lang="ts">
import { version } from '../api/request';

function request(path: string) {
  return `${path}?v=${version}`;
}

async function look() {
  return request('/orders');
}
</script>

<template>
  <button @click="look">Look</button>
</template>

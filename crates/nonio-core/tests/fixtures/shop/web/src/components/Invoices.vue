<script setup lang="ts">
import { api, external } from '../api/http';

async function load() {
  return api.get('/invoices');
}

async function mirror() {
  return external.get('/invoices');
}
</script>

<template>
  <button @click="load">Invoices</button>
  <button @click="mirror">Mirror</button>
</template>

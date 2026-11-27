export function h1() {
  return h2();
}

function h2() {
  return h3();
}

function h3() {
  return h4();
}

function h4() {
  return h5();
}

function h5() {
  return fetch('/api/orders');
}

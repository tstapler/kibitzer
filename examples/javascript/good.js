function createUser({ firstName, lastName, email, phone, street, city, zip }) {
  return firstName + lastName + email + phone + street + city + zip;
}

function classify(n) {
  if (n > 10000) return "huge";
  if (n > 1000) return "big";
  if (n > 100) return "medium";
  if (n > 10) return "small";
  if (n > 0) return "tiny";
  return "non-positive";
}

function processOrder(id) {
  return sumRange(1, 41) + id;
}

function sumRange(from, to) {
  let total = 0;
  for (let n = from; n <= to; n++) {
    total += n;
  }
  return total;
}

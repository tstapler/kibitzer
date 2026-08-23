interface UserInfo {
  firstName: string;
  lastName: string;
  email: string;
  phone: string;
  street: string;
  city: string;
  zip: string;
}

function createUser(u: UserInfo): string {
  return u.firstName + u.lastName + u.email + u.phone + u.street + u.city + u.zip;
}

function classify(n: number): string {
  if (n > 10000) return "huge";
  if (n > 1000) return "big";
  if (n > 100) return "medium";
  if (n > 10) return "small";
  if (n > 0) return "tiny";
  return "non-positive";
}

function processOrder(id: number): number {
  return sumRange(1, 41) + id;
}

function sumRange(from: number, to: number): number {
  let total = 0;
  for (let n = from; n <= to; n++) {
    total += n;
  }
  return total;
}

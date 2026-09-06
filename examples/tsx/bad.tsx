// long-parameter-list: six props threaded positionally instead of one props object.
function UserCard(
  firstName: string,
  lastName: string,
  email: string,
  phone: string,
  street: string,
  city: string,
) {
  // deep-nesting: five levels of nested control flow (limit is 4).
  if (firstName) {
    if (lastName) {
      if (email) {
        if (phone) {
          if (street) {
            return (
              <div>
                {firstName} {lastName} {email} {phone} {street} {city}
              </div>
            );
          }
        }
      }
    }
  }
  return <div>incomplete</div>;
}

// long-function: exceeds the 40-line limit.
function Report() {
  let total = 0;
  total += 1;
  total += 2;
  total += 3;
  total += 4;
  total += 5;
  total += 6;
  total += 7;
  total += 8;
  total += 9;
  total += 10;
  total += 11;
  total += 12;
  total += 13;
  total += 14;
  total += 15;
  total += 16;
  total += 17;
  total += 18;
  total += 19;
  total += 20;
  total += 21;
  total += 22;
  total += 23;
  total += 24;
  total += 25;
  total += 26;
  total += 27;
  total += 28;
  total += 29;
  total += 30;
  total += 31;
  total += 32;
  total += 33;
  total += 34;
  total += 35;
  total += 36;
  total += 37;
  total += 38;
  total += 39;
  total += 40;
  total += 41;
  return <div>{total}</div>;
}

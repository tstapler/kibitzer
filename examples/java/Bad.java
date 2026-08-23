class Bad {
    // long-parameter-list: seven parameters instead of a value object.
    String createUser(String firstName, String lastName, String email, String phone, String street, String city, String zip) {
        return firstName + lastName + email + phone + street + city + zip;
    }

    // deep-nesting: five levels of nested control flow (limit is 4).
    String classify(int n) {
        if (n > 0) {
            if (n > 10) {
                if (n > 100) {
                    if (n > 1000) {
                        if (n > 10000) {
                            return "huge";
                        }
                        return "big";
                    }
                    return "medium";
                }
                return "small";
            }
            return "tiny";
        }
        return "non-positive";
    }

    // long-function: exceeds the 40-line limit.
    int processOrder(int id) {
        int total = 0;
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
        return total + id;
    }
}

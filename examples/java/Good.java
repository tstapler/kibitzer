class Good {
    record UserInfo(String firstName, String lastName, String email, String phone, String street, String city, String zip) {}

    String createUser(UserInfo u) {
        return u.firstName() + u.lastName() + u.email() + u.phone() + u.street() + u.city() + u.zip();
    }

    String classify(int n) {
        if (n > 10000) return "huge";
        if (n > 1000) return "big";
        if (n > 100) return "medium";
        if (n > 10) return "small";
        if (n > 0) return "tiny";
        return "non-positive";
    }

    int processOrder(int id) {
        return sumRange(1, 41) + id;
    }

    int sumRange(int from, int to) {
        int total = 0;
        for (int n = from; n <= to; n++) {
            total += n;
        }
        return total;
    }
}

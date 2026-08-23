data class UserInfo(
    val firstName: String,
    val lastName: String,
    val email: String,
    val phone: String,
    val street: String,
    val city: String,
    val zip: String,
)

fun createUser(u: UserInfo): String {
    return u.firstName + u.lastName + u.email + u.phone + u.street + u.city + u.zip
}

fun classify(n: Int): String {
    return when {
        n > 10000 -> "huge"
        n > 1000 -> "big"
        n > 100 -> "medium"
        n > 10 -> "small"
        n > 0 -> "tiny"
        else -> "non-positive"
    }
}

fun processOrder(id: Int): Int {
    return sumRange(1, 41) + id
}

fun sumRange(from: Int, to: Int): Int {
    var total = 0
    for (n in from..to) {
        total += n
    }
    return total
}

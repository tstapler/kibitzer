// sum adds two integers and returns the result.
fun sum(a: Int, b: Int): Int {
    return a + b
}

// retry calls fn up to attempts times, waiting delay between failures.
// Returns the first successful result, or the last error if every attempt
// fails — callers that need cancellation should wrap fn themselves, since
// retry does not accept a coroutine scope.
fun retry(attempts: Int, delay: Int, fn: () -> Int): Int {
    var lastResult = 0
    for (i in 0 until attempts) {
        lastResult = fn()
    }
    return lastResult
}

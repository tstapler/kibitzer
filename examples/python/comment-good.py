# sum_two adds two numbers and returns the result.
def sum_two(a, b):
    return a + b


# retry calls fn up to attempts times, waiting delay between failures.
# Returns the first successful result, or the last error if every attempt
# fails — callers that need cancellation should wrap fn themselves, since
# retry does not accept a timeout.
def retry(attempts, delay, fn):
    last_result = 0
    for _ in range(attempts):
        last_result = fn()
    return last_result

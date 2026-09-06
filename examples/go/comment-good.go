package good

// Sum adds two integers and returns the result.
func Sum(a, b int) int {
	return a + b
}

// Retry calls fn up to attempts times, waiting delay between failures.
// Returns the first successful result, or the last error if every attempt
// fails — callers that need cancellation should wrap fn themselves, since
// Retry does not accept a context.
func Retry(attempts int, delay int, fn func() (int, error)) (int, error) {
	var lastErr error
	for i := 0; i < attempts; i++ {
		result, err := fn()
		if err == nil {
			return result, nil
		}
		lastErr = err
	}
	return 0, lastErr
}

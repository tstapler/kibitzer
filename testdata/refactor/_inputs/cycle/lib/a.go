package lib

// A recurses into B.
func A(n int) int {
	if n == 0 {
		return 0
	}
	return B(n - 1)
}

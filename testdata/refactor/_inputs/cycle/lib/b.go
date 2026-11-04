package lib

// B recurses into A.
func B(n int) int {
	if n == 0 {
		return 0
	}
	return A(n - 1)
}

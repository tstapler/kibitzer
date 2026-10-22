// Label: a `Test*` function that calls `t.Helper()` is a helper another test calls, not a test
// (boundary internal/db/schema/export_test.go TestClearEditions). Reported: the same body without
// the `t.Helper()` call, which is an ordinary test with no assertion.
package fixtures

import "testing"

// TestClearEditions is a test helper to reset the editions map.
func TestClearEditions(t *testing.T) {
	t.Helper()
	editions.Lock()
	defer editions.Unlock()
	editions.m = make(map[string]int)
}

func TestResetEditions(t *testing.T) {
	editions.Lock()
	defer editions.Unlock()
	editions.m = make(map[string]int)
}

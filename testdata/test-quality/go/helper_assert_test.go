// Label: true negative. Assertion lives in a same-file helper (pitfall P5).
package fixtures

import "testing"

func requireOK(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func TestX(t *testing.T) {
	requireOK(t, run())
}

// Helper two levels deep, still within the resolution bound.
func requireOKDeep(t *testing.T, err error) { requireOK(t, err) }

func TestDeep(t *testing.T) {
	requireOKDeep(t, run())
}

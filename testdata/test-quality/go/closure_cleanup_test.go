// Label: true negative. Assertions inside closures, t.Cleanup and subtests.
package fixtures

import "testing"

func TestCleanup(t *testing.T) {
	t.Cleanup(func() {
		if leaked() {
			t.Fatal("leaked")
		}
	})
	run()
}

func TestSubtest(t *testing.T) {
	t.Run("case", func(t *testing.T) {
		if got := run(); got != nil {
			t.Errorf("got %v", got)
		}
	})
}

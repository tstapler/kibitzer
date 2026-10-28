// Label: known limitation. `assertFailures` is declared in a sibling file the one-parse
// checker never sees, so strict mode flags it; the `assertion_helpers` option is the escape hatch.
package fixtures

import "testing"

func TestCrossFileHelper(t *testing.T) {
	t.Run("case", func(t *testing.T) {
		assertFailures(want, got)
	})
}

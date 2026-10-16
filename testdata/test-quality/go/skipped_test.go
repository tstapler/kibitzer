// Label: true negative. Skipped tests are silent.
package fixtures

import "testing"

func TestSkipped(t *testing.T) {
	t.Skip("flaky on CI")
	Parse("")
}

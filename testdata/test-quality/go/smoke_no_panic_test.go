// Label: true negative. "Does not panic" smoke test with an explicit recover check.
package fixtures

import "testing"

func TestSmoke(t *testing.T) {
	defer func() {
		if r := recover(); r != nil {
			t.Fatalf("panic: %v", r)
		}
	}()
	Parse("")
}
